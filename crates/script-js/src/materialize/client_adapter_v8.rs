use super::{js_i32, js_to_string, object_i32, set_key, v8_str, PENDING};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub(super) fn read<'s>(
    scope: &mut v8::HandleScope<'s>,
    method: v8::Local<'s, v8::Value>,
    arg0: v8::Local<'s, v8::Value>,
    arg1: v8::Local<'s, v8::Value>,
    arg2: v8::Local<'s, v8::Value>,
    arg3: v8::Local<'s, v8::Value>,
    arg4: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let method = js_to_string(scope, method)?;
    match method.as_str() {
        "skillCount" => {
            return Ok(v8::Integer::new(scope, api::snapshot::stat_count() as i32).into())
        }
        "skillUsed" => return skill_used(scope, arg0),
        "skillNames" => return skill_names(scope),
        "skillUsedMask" => return skill_used_mask(scope),
        _ => {}
    }
    let snapshot = snapshot(scope)?;
    let facts = property(scope, snapshot, "client_adapter")?;
    match method.as_str() {
        "worldTile" => world_tile(scope, snapshot),
        "serverTile" => property(scope, facts, "server_tile"),
        "selfSlot" => number_property(scope, snapshot, "self_slot", -1),
        "inventorySize" => number_property(scope, snapshot, "inv_size", 0),
        "inventory" => item_rows_property(scope, snapshot, "inv"),
        "bankSideItems" => item_rows_property(scope, snapshot, "bank_side"),
        "bankItems" => {
            let component = object_i32(scope, facts, "bank_com_id")?.unwrap_or(-1);
            if component >= 0 {
                item_rows_property(scope, snapshot, "bank")
            } else {
                Ok(empty_array(scope).into())
            }
        }
        "sceneState" => number_property(scope, snapshot, "scene_state", 0),
        "ingame" => bool_property_value(scope, snapshot, "ingame"),
        "npcs" => property(scope, snapshot, "npcs"),
        "locs" => property(scope, snapshot, "locs"),
        "players" => property(scope, snapshot, "players"),
        "groundItems" => property(scope, snapshot, "ground"),
        "equipment" => item_rows_property(scope, snapshot, "equipment"),
        "modals" => modals(scope, snapshot),
        "chatContinueComId" => number_property(scope, facts, "chat_continue_component_id", -1),
        "chatOptions" => chat_options(scope, snapshot),
        "chatModalTexts" => modal_texts(
            scope,
            facts,
            "chat_modal_texts",
            Some((snapshot, "chat_modal_id")),
        ),
        "activeSideTab" => number_property(scope, snapshot, "side_tab", -1),
        "localPlayerName" => nullable_string_property(scope, snapshot, "my_name"),
        "combatLevel" => combat_level(scope, snapshot),
        "selfChat" => nullable_string_property(scope, snapshot, "self_chat"),
        "hintTile" => hint_tile(scope, snapshot, facts),
        "npcBox" => npc_box(scope, snapshot, arg0),
        "retaliateControls" => property(scope, snapshot, "retaliate_controls"),
        "inCombat" => bool_property_value(scope, snapshot, "in_combat"),
        "selfAnim" => number_property(scope, snapshot, "self_anim", -1),
        "selfFaceEntity" => self_face_entity(scope, snapshot),
        "energy" => number_property(scope, snapshot, "run_energy", 0),
        "varp" => varp(scope, snapshot, arg0),
        "stat" => stat(scope, snapshot, arg0),
        "statsReady" => bool_property_value(scope, snapshot, "stats_ready"),
        "weight" => number_property(scope, snapshot, "weight", 0),
        "chat" => chat(scope, snapshot, arg0),
        "playerCount" => number_property(scope, facts, "player_count", 0),
        "npcCount" => number_property(scope, facts, "npc_count", 0),
        "cameraYaw" => camera_yaw(scope, snapshot),
        "cameraPitch" => number_property(scope, snapshot, "camera_pitch", 128),
        "takingDamage" => bool_property_value(scope, snapshot, "taking_damage"),
        "inventorySnapshotReady" => bool_property_value(scope, snapshot, "inv_ready"),
        "bankComId" => number_property(scope, facts, "bank_com_id", -1),
        "bankSnapshotReady" => bank_snapshot_ready(scope, snapshot, facts),
        "bankSideSnapshotReady" => bank_side_snapshot_ready(scope, snapshot),
        "bankSnapshotGeneration" => bank_snapshot_generation(scope, snapshot, facts),
        "runControls" => run_controls(scope, facts),
        "toWorld" => to_world(scope, snapshot, facts, arg0, arg1),
        "toLocal" => to_local(scope, snapshot, facts, arg0, arg1),
        "collisionFlags" => collision_flags(scope, snapshot, facts, arg0, arg1),
        "lastWalkPathWorld" => last_walk_path_world(scope, facts, snapshot),
        "loginMessage" => string_property(scope, facts, "login_message"),
        "menuEntries" => string_array_property(scope, facts, "menu_entries"),
        "shopInv" => shop_inventory(scope, facts, arg0),
        "puzzleBoardComId" => puzzle_board_field(scope, snapshot, "component_id", -1),
        "puzzleBoard" => puzzle_board_items(scope, snapshot),
        "puzzleBoardSize" => puzzle_board_field(scope, snapshot, "size", 0),
        "duelOffers" => duel_offers(scope, snapshot, facts),
        "tradeOfferOpen" => bool_property_value(scope, snapshot, "trade_offer_open"),
        "tradeConfirmOpen" => bool_property_value(scope, snapshot, "trade_confirm_open"),
        "tradeMyOffer" => item_rows_property(scope, snapshot, "trade_mine"),
        "tradeTheirOffer" => item_rows_property(scope, snapshot, "trade_theirs"),
        "tradeSidePack" => item_rows_property(scope, snapshot, "trade_side"),
        "tradePartner" => nullable_string_property(scope, snapshot, "trade_partner"),
        "tradeConfirmOffers" => trade_confirm_offers(scope, facts),
        "tradeConfirmReady" => trade_confirm_ready(scope, facts),
        "closeButtonComId" => close_button(scope, facts, arg0),
        "mainModalTexts" => modal_texts(
            scope,
            snapshot,
            "main_modal_texts",
            Some((snapshot, "main_modal_id")),
        ),
        "ifModelObjId" => if_model_obj_id(scope, facts, arg0),
        "modalButtons" => modal_buttons(scope, facts, arg0),
        "buttonByText" => button_by_text(scope, facts, arg0, arg1),
        "targetButtonByBase" => target_button_by_base(scope, facts, arg0, arg1),
        "selectButtonByVarp" => select_button_by_varp(scope, snapshot, facts, arg0, arg1, arg2),
        "selectButtonLabelsByVarp" => {
            select_button_labels_by_varp(scope, snapshot, facts, arg0, arg1)
        }
        "mainModalButtonNearText" => main_modal_button_near_text(scope, snapshot, facts, arg0),
        "mainSkillMultiItems" => main_skill_multi_items(scope, snapshot),
        "questStatuses" => quest_statuses(scope, snapshot, facts),
        "sideTabInterface" => side_tab_interface(scope, snapshot, arg0),
        "attached" => bool_property_value(scope, facts, "attached"),
        "mapBuildBase" => map_build_base(scope, facts),
        "modalCloseObservation" => modal_close_observation(scope, snapshot, facts),
        "ifText" => if_text(scope, snapshot, arg0),
        "countDialogOpen" => bool_property_value(scope, snapshot, "count_dialog_open"),
        "makeProducts" => make_products(scope, snapshot),
        "overlayPosWorld" => overlay_pos_world(scope, facts, [arg0, arg1, arg2, arg3, arg4], false),
        "projectAreaGameWorld" => {
            overlay_pos_world(scope, facts, [arg0, arg1, arg2, arg3, arg4], true)
        }
        "locBox" => {
            scope
                .get_slot::<Arc<AtomicBool>>()
                .ok_or_else(|| "loc projection interest is not installed".to_string())?
                .store(true, Ordering::Release);
            loc_box(scope, snapshot, facts, arg0, arg1)
        }
        "itemIconPixels" => item_icon_pixels(scope, facts, arg0),
        _ => {
            let _ = (arg2, arg3, arg4);
            Err(format!("not impl: reader.{method}"))
        }
    }
}

fn snapshot<'s>(scope: &mut v8::HandleScope<'s>) -> Result<v8::Local<'s, v8::Value>, String> {
    let context = scope.get_current_context();
    let global: v8::Local<v8::Value> = context.global(scope).into();
    let host = property(scope, global, "__rs2b0t_host")?;
    property(scope, host, "snapshot")
}

fn world_tile<'s>(
    scope: &mut v8::HandleScope<'s>,
    snapshot: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let context = scope.get_current_context();
    let global: v8::Local<v8::Value> = context.global(scope).into();
    let host = property(scope, global, "__rs2b0t_host")?;
    let tile = property(scope, host, "tile")?;
    if !tile.is_null() && !tile.is_undefined() {
        return Ok(tile);
    }
    let tile = property(scope, snapshot, "here")?;
    if tile.is_null() || tile.is_undefined() {
        Ok(v8::null(scope).into())
    } else {
        Ok(tile)
    }
}

fn property<'s>(
    scope: &mut v8::HandleScope<'s>,
    value: v8::Local<'s, v8::Value>,
    key: &str,
) -> Result<v8::Local<'s, v8::Value>, String> {
    if value.is_null() || value.is_undefined() {
        return Ok(v8::undefined(scope).into());
    }
    let object = value.to_object(scope).ok_or_else(|| PENDING.to_string())?;
    let key = v8_str(scope, key)?;
    object.get(scope, key).ok_or_else(|| PENDING.to_string())
}

fn array(value: v8::Local<v8::Value>) -> Option<v8::Local<v8::Array>> {
    v8::Local::<v8::Array>::try_from(value).ok()
}

fn array_value<'s>(
    scope: &mut v8::HandleScope<'s>,
    value: v8::Local<'s, v8::Value>,
    index: u32,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let Some(array) = array(value) else {
        return Ok(v8::undefined(scope).into());
    };
    array
        .get_index(scope, index)
        .ok_or_else(|| PENDING.to_string())
}

fn empty_array<'s>(scope: &mut v8::HandleScope<'s>) -> v8::Local<'s, v8::Array> {
    v8::Array::new(scope, 0)
}

fn set_i32(scope: &mut v8::HandleScope<'_>, object: v8::Local<v8::Object>, key: &str, value: i32) {
    let value = v8::Integer::new(scope, value);
    set_key(scope, object, key, value.into());
}

fn set_bool(
    scope: &mut v8::HandleScope<'_>,
    object: v8::Local<v8::Object>,
    key: &str,
    value: bool,
) {
    let value = v8::Boolean::new(scope, value);
    set_key(scope, object, key, value.into());
}

fn set_number(
    scope: &mut v8::HandleScope<'_>,
    object: v8::Local<v8::Object>,
    key: &str,
    value: f64,
) {
    let value = v8::Number::new(scope, value);
    set_key(scope, object, key, value.into());
}
fn number_property<'s>(
    scope: &mut v8::HandleScope<'s>,
    object: v8::Local<'s, v8::Value>,
    key: &str,
    fallback: i32,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let value = property(scope, object, key)?;
    let value = js_i32(scope, value)?.unwrap_or(fallback);
    Ok(v8::Integer::new(scope, value).into())
}
fn combat_level<'s>(
    scope: &mut v8::HandleScope<'s>,
    snapshot: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    if !bool_property(scope, snapshot, "ingame")? {
        return Ok(v8::Integer::new(scope, 0).into());
    }
    number_property(scope, snapshot, "combat_level", 0)
}

fn bool_property<'s>(
    scope: &mut v8::HandleScope<'s>,
    object: v8::Local<'s, v8::Value>,
    key: &str,
) -> Result<bool, String> {
    Ok(property(scope, object, key)?.is_true())
}

fn bool_property_value<'s>(
    scope: &mut v8::HandleScope<'s>,
    object: v8::Local<'s, v8::Value>,
    key: &str,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let value = bool_property(scope, object, key)?;
    Ok(v8::Boolean::new(scope, value).into())
}

fn string_property<'s>(
    scope: &mut v8::HandleScope<'s>,
    object: v8::Local<'s, v8::Value>,
    key: &str,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let value = property(scope, object, key)?;
    if value.is_string() {
        Ok(value)
    } else {
        Ok(v8::String::empty(scope).into())
    }
}

fn nullable_string_property<'s>(
    scope: &mut v8::HandleScope<'s>,
    object: v8::Local<'s, v8::Value>,
    key: &str,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let value = property(scope, object, key)?;
    if value.is_string() {
        Ok(value)
    } else {
        Ok(v8::null(scope).into())
    }
}

fn copy_string_array<'s>(
    scope: &mut v8::HandleScope<'s>,
    source: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let Some(source) = array(source) else {
        return Ok(empty_array(scope).into());
    };
    let output = v8::Array::new(scope, source.length() as i32);
    for index in 0..source.length() {
        let item = source
            .get_index(scope, index)
            .ok_or_else(|| PENDING.to_string())?;
        let copied = if item.is_string() {
            item
        } else {
            v8::String::empty(scope).into()
        };
        output
            .set_index(scope, index, copied)
            .ok_or_else(|| PENDING.to_string())?;
    }
    Ok(output.into())
}

fn item_rows<'s>(
    scope: &mut v8::HandleScope<'s>,
    source: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let Some(source) = array(source) else {
        return Ok(empty_array(scope).into());
    };
    let output = v8::Array::new(scope, 0);
    let mut out_index = 0;
    for index in 0..source.length() {
        let row = source
            .get_index(scope, index)
            .ok_or_else(|| PENDING.to_string())?;
        if !row.is_object() || row.is_array() {
            continue;
        }
        let slot = object_i32(scope, row, "slot")?.unwrap_or(-1);
        let id = object_i32(scope, row, "id")?.unwrap_or(-1);
        if slot < 0 || id < 0 {
            continue;
        }
        let count = object_i32(scope, row, "count")?.unwrap_or(0);
        let component = object_i32(scope, row, "component_id")?.unwrap_or(-1);
        let name = property(scope, row, "name")?;
        let name = if name.is_string() {
            name
        } else {
            v8::null(scope).into()
        };
        let ops_value = property(scope, row, "ops")?;
        let ops = copy_any_array(scope, ops_value)?;
        let item = v8::Object::new(scope);
        set_i32(scope, item, "slot", slot);
        set_i32(scope, item, "id", id);
        set_key(scope, item, "name", name);
        set_i32(scope, item, "count", count);
        set_key(scope, item, "ops", ops);
        set_i32(scope, item, "comId", component);
        output
            .set_index(scope, out_index, item.into())
            .ok_or_else(|| PENDING.to_string())?;
        out_index += 1;
    }
    Ok(output.into())
}

fn copy_any_array<'s>(
    scope: &mut v8::HandleScope<'s>,
    source: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let Some(source) = array(source) else {
        return Ok(empty_array(scope).into());
    };
    let output = v8::Array::new(scope, source.length() as i32);
    for index in 0..source.length() {
        let value = source
            .get_index(scope, index)
            .ok_or_else(|| PENDING.to_string())?;
        output
            .set_index(scope, index, value)
            .ok_or_else(|| PENDING.to_string())?;
    }
    Ok(output.into())
}

fn item_rows_property<'s>(
    scope: &mut v8::HandleScope<'s>,
    object: v8::Local<'s, v8::Value>,
    key: &str,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let source = property(scope, object, key)?;
    item_rows(scope, source)
}

fn string_array_property<'s>(
    scope: &mut v8::HandleScope<'s>,
    object: v8::Local<'s, v8::Value>,
    key: &str,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let source = property(scope, object, key)?;
    copy_string_array(scope, source)
}

fn modals<'s>(
    scope: &mut v8::HandleScope<'s>,
    snapshot: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let object = v8::Object::new(scope);
    for (key, source) in [
        ("main", "main_modal_id"),
        ("chat", "chat_modal_id"),
        ("side", "side_modal_id"),
    ] {
        let value = property(scope, snapshot, source)?;
        let value = js_i32(scope, value)?.unwrap_or(-1);
        set_i32(scope, object, key, value);
    }
    Ok(object.into())
}

fn chat_options<'s>(
    scope: &mut v8::HandleScope<'s>,
    snapshot: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let source = property(scope, snapshot, "chat_options")?;
    let Some(source) = array(source) else {
        return Ok(empty_array(scope).into());
    };
    let output = v8::Array::new(scope, source.length() as i32);
    for index in 0..source.length() {
        let row = source
            .get_index(scope, index)
            .ok_or_else(|| PENDING.to_string())?;
        let text = property(scope, row, "text")?;
        let id = property(scope, row, "comId")?;
        let id = js_i32(scope, id)?.unwrap_or(-1);
        let result = v8::Object::new(scope);
        set_key(scope, result, "text", text);
        set_i32(scope, result, "comId", id);
        output
            .set_index(scope, index, result.into())
            .ok_or_else(|| PENDING.to_string())?;
    }
    Ok(output.into())
}

fn modal_texts<'s>(
    scope: &mut v8::HandleScope<'s>,
    owner: v8::Local<'s, v8::Value>,
    key: &str,
    root_owner: Option<(v8::Local<'s, v8::Value>, &str)>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let pair = property(scope, owner, key)?;
    if pair.is_null() || pair.is_undefined() {
        return Ok(empty_array(scope).into());
    }
    if let Some((root_owner, root_key)) = root_owner {
        let expected = object_i32(scope, root_owner, root_key)?.unwrap_or(-1);
        let root = object_i32(scope, pair, "root")?.unwrap_or(-1);
        if root != expected || expected < 0 {
            return Ok(empty_array(scope).into());
        }
    }
    let texts = property(scope, pair, "texts")?;
    copy_string_array(scope, texts)
}

fn hint_tile<'s>(
    scope: &mut v8::HandleScope<'s>,
    snapshot: v8::Local<'s, v8::Value>,
    facts: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let tile = property(scope, snapshot, "hint_tile")?;
    if tile.is_null() || tile.is_undefined() {
        return Ok(v8::null(scope).into());
    }
    let Some(x) = object_i32(scope, tile, "x")? else {
        return Ok(v8::null(scope).into());
    };
    let Some(z) = object_i32(scope, tile, "z")? else {
        return Ok(v8::null(scope).into());
    };
    let here = property(scope, snapshot, "here")?;
    let here_level = object_i32(scope, here, "level")?;
    let facts_level = if facts.is_null() || facts.is_undefined() {
        None
    } else {
        object_i32(scope, facts, "hint_tile_level")?
    };
    let level = facts_level.or(here_level).unwrap_or(0);
    tile_object(scope, x, z, level)
}

fn tile_object<'s>(
    scope: &mut v8::HandleScope<'s>,
    x: i32,
    z: i32,
    level: i32,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let object = v8::Object::new(scope);
    set_i32(scope, object, "x", x);
    set_i32(scope, object, "z", z);
    set_i32(scope, object, "level", level);
    Ok(object.into())
}

fn npc_box<'s>(
    scope: &mut v8::HandleScope<'s>,
    snapshot: v8::Local<'s, v8::Value>,
    index: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let Some(index) = js_i32(scope, index)? else {
        return Ok(v8::null(scope).into());
    };
    if index < 0 {
        return Ok(v8::null(scope).into());
    }
    let rows = property(scope, snapshot, "npc_boxes")?;
    let Some(rows) = array(rows) else {
        return Ok(v8::null(scope).into());
    };
    let mut found = None;
    for at in 0..rows.length() {
        let row = rows
            .get_index(scope, at)
            .ok_or_else(|| PENDING.to_string())?;
        if object_i32(scope, row, "index")? == Some(index) {
            found = Some(row);
            break;
        }
    }
    let Some(row) = found else {
        return Ok(v8::null(scope).into());
    };
    let points = property(scope, row, "points")?;
    let Some(points) = array(points) else {
        return Ok(v8::null(scope).into());
    };
    if points.length() != 8 {
        return Ok(v8::null(scope).into());
    }
    let output = v8::Array::new(scope, 8);
    for at in 0..8 {
        let point = points
            .get_index(scope, at)
            .ok_or_else(|| PENDING.to_string())?;
        let Some(x) = object_i32(scope, point, "x")? else {
            return Ok(v8::null(scope).into());
        };
        let Some(y) = object_i32(scope, point, "y")? else {
            return Ok(v8::null(scope).into());
        };
        let p = v8::Object::new(scope);
        set_i32(scope, p, "x", x);
        set_i32(scope, p, "y", y);
        output
            .set_index(scope, at, p.into())
            .ok_or_else(|| PENDING.to_string())?;
    }
    Ok(output.into())
}

fn self_face_entity<'s>(
    scope: &mut v8::HandleScope<'s>,
    snapshot: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let kind = number_property(scope, snapshot, "self_target_kind", 0)?;
    let index = number_property(scope, snapshot, "self_target_index", -1)?;
    let kind = kind.int32_value(scope).unwrap_or(0);
    let index = index.int32_value(scope).unwrap_or(-1);
    let face = match kind {
        1 => index,
        2 => index.saturating_add(32_768),
        _ => -1,
    };
    Ok(v8::Integer::new(scope, face).into())
}

fn varp<'s>(
    scope: &mut v8::HandleScope<'s>,
    snapshot: v8::Local<'s, v8::Value>,
    index: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let Some(index) = js_i32(scope, index)? else {
        return Ok(v8::Integer::new(scope, 0).into());
    };
    let rows = property(scope, snapshot, "varps")?;
    let Some(rows) = array(rows) else {
        return Ok(v8::Integer::new(scope, 0).into());
    };
    for at in 0..rows.length() {
        let row = rows
            .get_index(scope, at)
            .ok_or_else(|| PENDING.to_string())?;
        if object_i32(scope, row, "index")? == Some(index) {
            let value = object_i32(scope, row, "value")?.unwrap_or(0);
            return Ok(v8::Integer::new(scope, value).into());
        }
    }
    Ok(v8::Integer::new(scope, 0).into())
}

fn stat<'s>(
    scope: &mut v8::HandleScope<'s>,
    snapshot: v8::Local<'s, v8::Value>,
    index: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let index_text = js_to_string(scope, index)?;
    let index = js_i32(scope, index)?;
    let rows = property(scope, snapshot, "stats")?;
    let row = match index {
        Some(index) if index >= 0 => array_value(scope, rows, index as u32)?,
        _ => v8::undefined(scope).into(),
    };
    let stat_name = index
        .and_then(|index| usize::try_from(index).ok())
        .map(api::snapshot::stat_name)
        .filter(|name| !name.is_empty());
    let name = match stat_name {
        Some(name) => v8_str(scope, name)?,
        None => v8_str(scope, &format!("#{index_text}"))?,
    };
    let (effective, base, xp) = if !row.is_null() && !row.is_undefined() && row.is_object() {
        (
            object_i32(scope, row, "effective")?.unwrap_or(0),
            object_i32(scope, row, "base")?.unwrap_or(0),
            object_i32(scope, row, "xp")?.unwrap_or(0),
        )
    } else {
        (0, 0, 0)
    };
    let result = v8::Object::new(scope);
    set_key(scope, result, "name", name);
    set_i32(scope, result, "effective", effective);
    set_i32(scope, result, "base", base);
    set_i32(scope, result, "xp", xp);
    Ok(result.into())
}

fn skill_used<'s>(
    scope: &mut v8::HandleScope<'s>,
    index: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let used = js_i32(scope, index)?
        .and_then(|index| usize::try_from(index).ok())
        .is_some_and(api::snapshot::stat_used);
    Ok(v8::Boolean::new(scope, used).into())
}
fn skill_names<'s>(scope: &mut v8::HandleScope<'s>) -> Result<v8::Local<'s, v8::Value>, String> {
    let count = api::snapshot::stat_count();
    let length =
        i32::try_from(count).map_err(|_| "skill count exceeds V8 array limit".to_string())?;
    let output = v8::Array::new(scope, length);
    for index in 0..count {
        let name = v8_str(scope, api::snapshot::stat_name(index))?;
        let index =
            u32::try_from(index).map_err(|_| "skill index exceeds V8 array limit".to_string())?;
        output
            .set_index(scope, index, name)
            .ok_or_else(|| PENDING.to_string())?;
    }
    Ok(output.into())
}

fn skill_used_mask<'s>(
    scope: &mut v8::HandleScope<'s>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let count = api::snapshot::stat_count();
    let length =
        i32::try_from(count).map_err(|_| "skill count exceeds V8 array limit".to_string())?;
    let output = v8::Array::new(scope, length);
    for index in 0..count {
        let used = v8::Boolean::new(scope, api::snapshot::stat_used(index));
        let index =
            u32::try_from(index).map_err(|_| "skill index exceeds V8 array limit".to_string())?;
        output
            .set_index(scope, index, used.into())
            .ok_or_else(|| PENDING.to_string())?;
    }
    Ok(output.into())
}

fn chat<'s>(
    scope: &mut v8::HandleScope<'s>,
    snapshot: v8::Local<'s, v8::Value>,
    count: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let count = count.number_value(scope).unwrap_or(f64::NAN);
    let limit = if count.is_nan() || count <= 0.0 {
        0
    } else if count.is_infinite() || count >= 100.0 {
        100
    } else {
        count.ceil() as u32
    };
    let lines = property(scope, snapshot, "chat_lines")?;
    let Some(lines) = array(lines) else {
        return Ok(empty_array(scope).into());
    };
    let output = v8::Array::new(scope, 0);
    for (out_index, index) in (0..limit.min(lines.length())).enumerate() {
        let line = lines
            .get_index(scope, index)
            .ok_or_else(|| PENDING.to_string())?;
        let text = property(scope, line, "text")?;
        if text.is_null() {
            break;
        }
        let type_ = property(scope, line, "type")?;
        let username = property(scope, line, "username")?;
        let username = if username.is_null() || username.is_undefined() {
            v8::null(scope).into()
        } else {
            username
        };
        let result = v8::Object::new(scope);
        set_key(scope, result, "type", type_);
        set_key(scope, result, "username", username);
        set_key(scope, result, "text", text);
        output
            .set_index(scope, out_index as u32, result.into())
            .ok_or_else(|| PENDING.to_string())?;
    }
    Ok(output.into())
}

fn camera_yaw<'s>(
    scope: &mut v8::HandleScope<'s>,
    snapshot: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let yaw = object_i32(scope, snapshot, "camera_yaw")?.unwrap_or(0) & 0x7ff;
    Ok(v8::Integer::new(scope, yaw).into())
}

fn bank_snapshot_ready<'s>(
    scope: &mut v8::HandleScope<'s>,
    snapshot: v8::Local<'s, v8::Value>,
    facts: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let open = bool_property(scope, snapshot, "bank_loaded")?;
    let component = object_i32(scope, facts, "bank_com_id")?.unwrap_or(-1);
    Ok(v8::Boolean::new(scope, open && component >= 0).into())
}

fn bank_side_snapshot_ready<'s>(
    scope: &mut v8::HandleScope<'s>,
    snapshot: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let ready = bool_property(scope, snapshot, "bank_side_ready")?;
    let side = object_i32(scope, snapshot, "side_modal_id")?.unwrap_or(-1);
    Ok(v8::Boolean::new(scope, ready && side >= 0).into())
}

fn bank_snapshot_generation<'s>(
    scope: &mut v8::HandleScope<'s>,
    snapshot: v8::Local<'s, v8::Value>,
    facts: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let component = object_i32(scope, facts, "bank_com_id")?.unwrap_or(-1);
    let open = bool_property(scope, snapshot, "bank_open")?;
    let generation = object_i32(scope, snapshot, "bank_snapshot_generation")?.unwrap_or(-1);
    Ok(v8::Integer::new(
        scope,
        if component >= 0 && open {
            generation
        } else {
            -1
        },
    )
    .into())
}

fn run_controls<'s>(
    scope: &mut v8::HandleScope<'s>,
    facts: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let on = object_i32(scope, facts, "run_on_com_id")?.unwrap_or(-1);
    let off = object_i32(scope, facts, "run_off_com_id")?.unwrap_or(-1);
    if on < 0 || off < 0 {
        return Ok(v8::null(scope).into());
    }
    let result = v8::Object::new(scope);
    set_i32(scope, result, "onComId", on);
    set_i32(scope, result, "offComId", off);
    Ok(result.into())
}

fn map_build_base<'s>(
    scope: &mut v8::HandleScope<'s>,
    facts: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    if !bool_property(scope, facts, "attached")? {
        return Ok(v8::null(scope).into());
    }
    let base = property(scope, facts, "map_build_base")?;
    let Some(x) = object_i32(scope, base, "x")? else {
        return Ok(v8::null(scope).into());
    };
    let Some(z) = object_i32(scope, base, "z")? else {
        return Ok(v8::null(scope).into());
    };
    let result = v8::Object::new(scope);
    set_i32(scope, result, "x", x);
    set_i32(scope, result, "z", z);
    Ok(result.into())
}

fn to_world<'s>(
    scope: &mut v8::HandleScope<'s>,
    snapshot: v8::Local<'s, v8::Value>,
    facts: v8::Local<'s, v8::Value>,
    x: v8::Local<'s, v8::Value>,
    z: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    if !bool_property(scope, facts, "attached")? {
        return Ok(v8::null(scope).into());
    }
    let Some(x) = js_i32(scope, x)? else {
        return Ok(v8::null(scope).into());
    };
    let Some(z) = js_i32(scope, z)? else {
        return Ok(v8::null(scope).into());
    };
    let base = property(scope, facts, "map_build_base")?;
    let Some(base_x) = object_i32(scope, base, "x")? else {
        return Ok(v8::null(scope).into());
    };
    let Some(base_z) = object_i32(scope, base, "z")? else {
        return Ok(v8::null(scope).into());
    };
    let here = property(scope, snapshot, "here")?;
    let level = object_i32(scope, here, "level")?.unwrap_or(0);
    tile_object(
        scope,
        base_x.saturating_add(x),
        base_z.saturating_add(z),
        level,
    )
}

fn to_local<'s>(
    scope: &mut v8::HandleScope<'s>,
    _snapshot: v8::Local<'s, v8::Value>,
    facts: v8::Local<'s, v8::Value>,
    x: v8::Local<'s, v8::Value>,
    z: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    if !bool_property(scope, facts, "attached")? {
        return Ok(v8::null(scope).into());
    }
    let Some(x) = js_i32(scope, x)? else {
        return Ok(v8::null(scope).into());
    };
    let Some(z) = js_i32(scope, z)? else {
        return Ok(v8::null(scope).into());
    };
    let base = property(scope, facts, "map_build_base")?;
    let Some(base_x) = object_i32(scope, base, "x")? else {
        return Ok(v8::null(scope).into());
    };
    let Some(base_z) = object_i32(scope, base, "z")? else {
        return Ok(v8::null(scope).into());
    };
    let lx = x - base_x;
    let lz = z - base_z;
    if !(0..104).contains(&lx) || !(0..104).contains(&lz) {
        return Ok(v8::null(scope).into());
    }
    let result = v8::Object::new(scope);
    set_i32(scope, result, "lx", lx);
    set_i32(scope, result, "lz", lz);
    Ok(result.into())
}

fn collision_flags<'s>(
    scope: &mut v8::HandleScope<'s>,
    snapshot: v8::Local<'s, v8::Value>,
    facts: v8::Local<'s, v8::Value>,
    lx: v8::Local<'s, v8::Value>,
    lz: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    if !bool_property(scope, facts, "attached")? {
        return Ok(v8::null(scope).into());
    }
    let Some(lx) = js_i32(scope, lx)? else {
        return Ok(v8::null(scope).into());
    };
    let Some(lz) = js_i32(scope, lz)? else {
        return Ok(v8::null(scope).into());
    };
    let collision = property(scope, snapshot, "collision")?;
    if !bool_property(scope, collision, "available")? {
        return Ok(v8::null(scope).into());
    }
    let width = object_i32(scope, collision, "width")?.unwrap_or(0);
    let height = object_i32(scope, collision, "height")?.unwrap_or(0);
    if lx < 0 || lz < 0 || lx >= width || lz >= height {
        return Ok(v8::null(scope).into());
    }
    let flags = property(scope, collision, "flags")?;
    if flags.is_null() || flags.is_undefined() {
        return Ok(v8::null(scope).into());
    }
    let Some(index) = u32::try_from(i64::from(lx) * i64::from(height) + i64::from(lz)).ok() else {
        return Ok(v8::null(scope).into());
    };
    let at = property(scope, flags, "at")?;
    let Ok(at) = v8::Local::<v8::Function>::try_from(at) else {
        return Ok(v8::null(scope).into());
    };
    let index = v8::Integer::new_from_unsigned(scope, index);
    let Some(value) = at.call(scope, flags, &[index.into()]) else {
        return Err(PENDING.to_string());
    };
    if value.is_number() {
        Ok(value)
    } else {
        Ok(v8::null(scope).into())
    }
}

fn last_walk_path_world<'s>(
    scope: &mut v8::HandleScope<'s>,
    facts: v8::Local<'s, v8::Value>,
    snapshot: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    if !bool_property(scope, facts, "attached")? {
        return Ok(empty_array(scope).into());
    }
    let path = property(scope, facts, "last_walk_path_local")?;
    let Some(path) = array(path) else {
        return Ok(empty_array(scope).into());
    };
    if path.length() == 0 {
        return Ok(empty_array(scope).into());
    }
    let base = property(scope, facts, "map_build_base")?;
    let base_x = object_i32(scope, base, "x")?.unwrap_or(0);
    let base_z = object_i32(scope, base, "z")?.unwrap_or(0);
    let server_tile = property(scope, facts, "server_tile")?;
    let here = property(scope, snapshot, "here")?;
    let here_level = object_i32(scope, here, "level")?;
    let level = object_i32(scope, server_tile, "level")?
        .or(here_level)
        .unwrap_or(0);
    let output = v8::Array::new(scope, (path.length() / 2) as i32);
    let mut at = 0;
    let mut index = 0;
    while index + 1 < path.length() {
        let x = path
            .get_index(scope, index)
            .ok_or_else(|| PENDING.to_string())?;
        let z = path
            .get_index(scope, index + 1)
            .ok_or_else(|| PENDING.to_string())?;
        let x = js_i32(scope, x)?.unwrap_or(0);
        let z = js_i32(scope, z)?.unwrap_or(0);
        let tile = tile_object(
            scope,
            base_x.saturating_add(x),
            base_z.saturating_add(z),
            level,
        )?;
        output
            .set_index(scope, at, tile)
            .ok_or_else(|| PENDING.to_string())?;
        at += 1;
        index += 2;
    }
    Ok(output.into())
}

fn widgets<'s>(
    scope: &mut v8::HandleScope<'s>,
    facts: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Array>, String> {
    let rows = property(scope, facts, "widgets")?;
    Ok(array(rows).unwrap_or_else(|| empty_array(scope)))
}

fn widget_by_id<'s>(
    scope: &mut v8::HandleScope<'s>,
    facts: v8::Local<'s, v8::Value>,
    component_id: i32,
) -> Result<Option<v8::Local<'s, v8::Value>>, String> {
    if component_id < 0 {
        return Ok(None);
    }
    let rows = widgets(scope, facts)?;
    for index in 0..rows.length() {
        let row = rows
            .get_index(scope, index)
            .ok_or_else(|| PENDING.to_string())?;
        if object_i32(scope, row, "component_id")? == Some(component_id) {
            return Ok(Some(row));
        }
    }
    Ok(None)
}

fn widget_rows_under<'s>(
    scope: &mut v8::HandleScope<'s>,
    facts: v8::Local<'s, v8::Value>,
    root_id: i32,
) -> Result<Vec<v8::Local<'s, v8::Value>>, String> {
    let rows = widgets(scope, facts)?;
    let mut result = Vec::new();
    for index in 0..rows.length() {
        let row = rows
            .get_index(scope, index)
            .ok_or_else(|| PENDING.to_string())?;
        if root_id < 0 || object_i32(scope, row, "root_component_id")? == Some(root_id) {
            result.push(row);
        }
    }
    Ok(result)
}

fn component_items<'s>(
    scope: &mut v8::HandleScope<'s>,
    facts: v8::Local<'s, v8::Value>,
    component_id: i32,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let Some(row) = widget_by_id(scope, facts, component_id)? else {
        return Ok(empty_array(scope).into());
    };
    item_rows_property(scope, row, "items")
}

fn shop_inventory<'s>(
    scope: &mut v8::HandleScope<'s>,
    facts: v8::Local<'s, v8::Value>,
    component_id: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let component_id = js_i32(scope, component_id)?.unwrap_or(-1);
    component_items(scope, facts, component_id)
}

fn puzzle_board_field<'s>(
    scope: &mut v8::HandleScope<'s>,
    snapshot: v8::Local<'s, v8::Value>,
    field: &str,
    fallback: i32,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let board = property(scope, snapshot, "puzzle_board")?;
    number_property(scope, board, field, fallback)
}

fn puzzle_board_items<'s>(
    scope: &mut v8::HandleScope<'s>,
    snapshot: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let board = property(scope, snapshot, "puzzle_board")?;
    item_rows_property(scope, board, "items")
}

fn side_tab_interface<'s>(
    scope: &mut v8::HandleScope<'s>,
    snapshot: v8::Local<'s, v8::Value>,
    tab: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let Some(tab) = js_i32(scope, tab)? else {
        return Ok(v8::Integer::new(scope, -1).into());
    };
    let rows = property(scope, snapshot, "side_tab_ifaces")?;
    let Some(rows) = array(rows) else {
        return Ok(v8::Integer::new(scope, -1).into());
    };
    for index in 0..rows.length() {
        let row = rows
            .get_index(scope, index)
            .ok_or_else(|| PENDING.to_string())?;
        if object_i32(scope, row, "index")? == Some(tab) {
            let id = object_i32(scope, row, "id")?.unwrap_or(-1);
            return Ok(v8::Integer::new(scope, id).into());
        }
    }
    Ok(v8::Integer::new(scope, -1).into())
}

fn if_text<'s>(
    scope: &mut v8::HandleScope<'s>,
    snapshot: v8::Local<'s, v8::Value>,
    component_id: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let Some(component_id) = js_i32(scope, component_id)? else {
        return Ok(v8::null(scope).into());
    };
    let Some(row) = widget_by_id(scope, snapshot, component_id)? else {
        return Ok(v8::null(scope).into());
    };
    let text = property(scope, row, "text")?;
    if text.is_string() {
        Ok(text)
    } else {
        Ok(v8::null(scope).into())
    }
}

fn close_button<'s>(
    scope: &mut v8::HandleScope<'s>,
    facts: v8::Local<'s, v8::Value>,
    root: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let root = js_i32(scope, root)?.unwrap_or(-1);
    for row in widget_rows_under(scope, facts, root)? {
        if object_i32(scope, row, "button_type")? == Some(3) {
            let component_id = object_i32(scope, row, "component_id")?.unwrap_or(-1);
            return Ok(v8::Integer::new(scope, component_id).into());
        }
    }
    Ok(v8::Integer::new(scope, -1).into())
}

fn if_model_obj_id<'s>(
    scope: &mut v8::HandleScope<'s>,
    facts: v8::Local<'s, v8::Value>,
    component: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let component = js_i32(scope, component)?.unwrap_or(-1);
    let Some(row) = widget_by_id(scope, facts, component)? else {
        return Ok(v8::null(scope).into());
    };
    if object_i32(scope, row, "model_type")? == Some(4) {
        let id = object_i32(scope, row, "model_id")?.unwrap_or(0);
        Ok(v8::Integer::new(scope, id).into())
    } else {
        Ok(v8::null(scope).into())
    }
}

fn button_by_text<'s>(
    scope: &mut v8::HandleScope<'s>,
    facts: v8::Local<'s, v8::Value>,
    root: v8::Local<'s, v8::Value>,
    label: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let root = js_i32(scope, root)?.unwrap_or(-1);
    let wanted = js_to_string(scope, label)?.to_lowercase();
    for row in widget_rows_under(scope, facts, root)? {
        let text = property(scope, row, "button_text")?;
        if text.is_string() && text.to_rust_string_lossy(scope).to_lowercase() == wanted {
            let component_id = object_i32(scope, row, "component_id")?.unwrap_or(-1);
            return Ok(v8::Integer::new(scope, component_id).into());
        }
    }
    Ok(v8::Integer::new(scope, -1).into())
}

fn target_button_by_base<'s>(
    scope: &mut v8::HandleScope<'s>,
    facts: v8::Local<'s, v8::Value>,
    root: v8::Local<'s, v8::Value>,
    base: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let root = js_i32(scope, root)?.unwrap_or(-1);
    let wanted = js_to_string(scope, base)?.to_lowercase();
    for row in widget_rows_under(scope, facts, root)? {
        if object_i32(scope, row, "button_type")? != Some(2) {
            continue;
        }
        let target = property(scope, row, "target_base")?;
        if target.is_string() && target.to_rust_string_lossy(scope).to_lowercase() == wanted {
            let component_id = object_i32(scope, row, "component_id")?.unwrap_or(-1);
            return Ok(v8::Integer::new(scope, component_id).into());
        }
    }
    Ok(v8::Integer::new(scope, -1).into())
}

/// The combat tab's posted style rows (`mode`, `label`, `component_id`) when
/// `root` is the combat tab, `varp` is its mode varp and its component tree is
/// not in the posted widget facts. The widget facts walk only the shown side
/// tab (`api` `widget_roots`), but upstream's reader reads the loaded combat
/// tab whichever tab is shown. Rust posts these rows from the combat tab's own
/// tree (`host-play` `script_snapshot.rs`, `combat_style_labels`).
fn combat_tab_style_rows<'s>(
    scope: &mut v8::HandleScope<'s>,
    snapshot: v8::Local<'s, v8::Value>,
    root: i32,
    varp: i32,
    posted_rows: usize,
) -> Result<Option<v8::Local<'s, v8::Array>>, String> {
    if posted_rows > 0
        || varp != COMBAT_MODE_VARP
        || root < 0
        || side_tab_interface_value(scope, snapshot, COMBAT_SIDE_TAB)? != root
    {
        return Ok(None);
    }
    let rows = property(scope, snapshot, "combat_styles")?;
    Ok(array(rows))
}

/// Content `varp` 43 (`com_mode`) selects the combat style, and side tab 0 is
/// the combat tab.
const COMBAT_MODE_VARP: i32 = 43;
const COMBAT_SIDE_TAB: i32 = 0;

fn select_button_by_varp<'s>(
    scope: &mut v8::HandleScope<'s>,
    snapshot: v8::Local<'s, v8::Value>,
    facts: v8::Local<'s, v8::Value>,
    root: v8::Local<'s, v8::Value>,
    varp: v8::Local<'s, v8::Value>,
    value: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let root = js_i32(scope, root)?.unwrap_or(-1);
    let varp = js_i32(scope, varp)?.unwrap_or(i32::MIN);
    let value = js_i32(scope, value)?.unwrap_or(i32::MIN);
    let rows = widget_rows_under(scope, facts, root)?;
    for row in &rows {
        if object_i32(scope, *row, "button_type")? == Some(5)
            && object_i32(scope, *row, "script_opcode")? == Some(5)
            && object_i32(scope, *row, "script_varp")? == Some(varp)
            && object_i32(scope, *row, "script_value")? == Some(value)
        {
            let component_id = object_i32(scope, *row, "component_id")?.unwrap_or(-1);
            return Ok(v8::Integer::new(scope, component_id).into());
        }
    }
    if let Some(styles) = combat_tab_style_rows(scope, snapshot, root, varp, rows.len())? {
        for index in 0..styles.length() {
            let style = styles
                .get_index(scope, index)
                .ok_or_else(|| PENDING.to_string())?;
            if object_i32(scope, style, "mode")? == Some(value) {
                let component_id = object_i32(scope, style, "component_id")?.unwrap_or(-1);
                return Ok(v8::Integer::new(scope, component_id).into());
            }
        }
    }
    Ok(v8::Integer::new(scope, -1).into())
}

fn select_button_labels_by_varp<'s>(
    scope: &mut v8::HandleScope<'s>,
    snapshot: v8::Local<'s, v8::Value>,
    facts: v8::Local<'s, v8::Value>,
    root: v8::Local<'s, v8::Value>,
    varp: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let root = js_i32(scope, root)?.unwrap_or(-1);
    let varp = js_i32(scope, varp)?.unwrap_or(i32::MIN);
    let rows = widget_rows_under(scope, facts, root)?;
    if let Some(styles) = combat_tab_style_rows(scope, snapshot, root, varp, rows.len())? {
        let output = v8::Array::new(scope, 0);
        let mut out_index = 0;
        for index in 0..styles.length() {
            let style = styles
                .get_index(scope, index)
                .ok_or_else(|| PENDING.to_string())?;
            let mode = object_i32(scope, style, "mode")?.unwrap_or(0);
            let label = property(scope, style, "label")?;
            if !label.is_string() {
                continue;
            }
            let item = v8::Object::new(scope);
            set_i32(scope, item, "mode", mode);
            set_key(scope, item, "label", label);
            output
                .set_index(scope, out_index, item.into())
                .ok_or_else(|| PENDING.to_string())?;
            out_index += 1;
        }
        return Ok(output.into());
    }
    let mut buttons = Vec::new();
    for row in &rows {
        if object_i32(scope, *row, "button_type")? == Some(5)
            && object_i32(scope, *row, "script_opcode")? == Some(5)
            && object_i32(scope, *row, "script_varp")? == Some(varp)
            && object_i32(scope, *row, "script_value")?.is_some()
        {
            buttons.push(*row);
        }
    }
    buttons.sort_by_key(|row| {
        (
            object_i32(scope, *row, "y").ok().flatten().unwrap_or(0),
            object_i32(scope, *row, "x").ok().flatten().unwrap_or(0),
            object_i32(scope, *row, "component_id")
                .ok()
                .flatten()
                .unwrap_or(-1),
        )
    });
    let output = v8::Array::new(scope, 0);
    let mut out_index = 0;
    for button in buttons {
        let parent = object_i32(scope, button, "parent_id")?.unwrap_or(-1);
        let y = object_i32(scope, button, "y")?.unwrap_or(0);
        let height = object_i32(scope, button, "height")?.unwrap_or(0);
        let centre = f64::from(y) + f64::from(height) / 2.0;
        let mut best: Option<(f64, String)> = None;
        for candidate in &rows {
            if object_i32(scope, *candidate, "parent_id")? != Some(parent)
                || object_i32(scope, *candidate, "type_")? != Some(4)
            {
                continue;
            }
            let text = property(scope, *candidate, "text")?;
            if !text.is_string() {
                continue;
            }
            let label = text.to_rust_string_lossy(scope);
            if label.trim().is_empty() {
                continue;
            }
            let text_y = f64::from(object_i32(scope, *candidate, "y")?.unwrap_or(0));
            let text_height = f64::from(object_i32(scope, *candidate, "height")?.unwrap_or(0));
            let text_centre = text_y + text_height / 2.0;
            if text_centre < f64::from(y) || text_centre > f64::from(y + height) {
                continue;
            }
            let distance = (text_centre - centre).abs();
            if best
                .as_ref()
                .is_none_or(|(best_distance, _)| distance < *best_distance)
            {
                best = Some((distance, label));
            }
        }
        if let Some((_, label)) = best {
            let mode = object_i32(scope, button, "script_value")?.unwrap_or(0);
            let label = v8_str(scope, &label)?;
            let item = v8::Object::new(scope);
            set_i32(scope, item, "mode", mode);
            set_key(scope, item, "label", label);
            output
                .set_index(scope, out_index, item.into())
                .ok_or_else(|| PENDING.to_string())?;
            out_index += 1;
        }
    }
    Ok(output.into())
}

fn main_modal_button_near_text<'s>(
    scope: &mut v8::HandleScope<'s>,
    snapshot: v8::Local<'s, v8::Value>,
    facts: v8::Local<'s, v8::Value>,
    label: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let root = object_i32(scope, snapshot, "main_modal_id")?.unwrap_or(-1);
    if root < 0 {
        return Ok(v8::Integer::new(scope, -1).into());
    }
    let wanted = js_to_string(scope, label)?.to_lowercase();
    let rows = widget_rows_under(scope, facts, root)?;
    let text = rows.iter().find(|row| {
        object_i32(scope, **row, "type_").ok().flatten() == Some(4)
            && property(scope, **row, "text").ok().is_some_and(|value| {
                value.is_string()
                    && value
                        .to_rust_string_lossy(scope)
                        .to_lowercase()
                        .contains(&wanted)
            })
    });
    let Some(text) = text else {
        return Ok(v8::Integer::new(scope, -1).into());
    };
    let text_x = object_i32(scope, *text, "x")?.unwrap_or(0);
    let text_y = object_i32(scope, *text, "y")?.unwrap_or(0);
    let text_w = object_i32(scope, *text, "width")?.unwrap_or(0);
    let text_h = object_i32(scope, *text, "height")?.unwrap_or(0);
    let tx = f64::from(text_x) + f64::from(text_w) / 2.0;
    let ty = f64::from(text_y) + f64::from(text_h) / 2.0;
    let mut closest: Option<(f64, i32)> = None;
    for row in rows {
        if !matches!(object_i32(scope, row, "button_type")?, Some(1 | 2 | 5)) {
            continue;
        }
        let x = f64::from(object_i32(scope, row, "x")?.unwrap_or(0))
            + f64::from(object_i32(scope, row, "width")?.unwrap_or(0)) / 2.0;
        let y = f64::from(object_i32(scope, row, "y")?.unwrap_or(0))
            + f64::from(object_i32(scope, row, "height")?.unwrap_or(0)) / 2.0;
        let distance = (x - tx).hypot(y - ty);
        let id = object_i32(scope, row, "component_id")?.unwrap_or(-1);
        if closest.is_none_or(|(best, _)| distance < best) {
            closest = Some((distance, id));
        }
    }
    Ok(v8::Integer::new(scope, closest.map_or(-1, |(_, id)| id)).into())
}

fn modal_buttons<'s>(
    scope: &mut v8::HandleScope<'s>,
    facts: v8::Local<'s, v8::Value>,
    root: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let root = js_i32(scope, root)?.unwrap_or(-1);
    let rows = widget_rows_under(scope, facts, root)?;
    let mut hidden = HashMap::with_capacity(rows.len());
    let output = v8::Array::new(scope, 0);
    let mut out_index = 0;
    for row in rows {
        let id = object_i32(scope, row, "component_id")?.unwrap_or(-1);
        let parent = object_i32(scope, row, "parent_id")?.unwrap_or(-1);
        let inherited = hidden.get(&parent).copied().unwrap_or(false);
        let is_hidden = inherited || bool_property(scope, row, "hidden")?;
        hidden.insert(id, is_hidden);
        let button_type = object_i32(scope, row, "button_type")?.unwrap_or(0);
        if button_type <= 0 || button_type == 3 {
            continue;
        }
        let text = property(scope, row, "text")?;
        let label = if text.is_string() {
            text
        } else {
            v8_str(scope, "")?
        };
        let menu = property(scope, row, "button_text")?;
        let menu = if menu.is_string() {
            menu
        } else {
            v8_str(scope, "")?
        };
        let item = v8::Object::new(scope);
        set_i32(scope, item, "comId", id);
        set_key(scope, item, "label", label);
        set_key(scope, item, "menu", menu);
        set_bool(scope, item, "hidden", is_hidden);
        set_bool(scope, item, "pause", button_type == 6);
        output
            .set_index(scope, out_index, item.into())
            .ok_or_else(|| PENDING.to_string())?;
        out_index += 1;
    }
    Ok(output.into())
}

fn trade_confirm_offers<'s>(
    scope: &mut v8::HandleScope<'s>,
    facts: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let result = v8::Object::new(scope);
    let mine_small = component_items(scope, facts, 3542)?;
    let mine_large = component_items(scope, facts, 3538)?;
    let their_small = component_items(scope, facts, 3532)?;
    let their_large = component_items(scope, facts, 3539)?;
    let mine = concat_arrays(scope, mine_small, mine_large)?;
    let theirs = concat_arrays(scope, their_small, their_large)?;
    set_key(scope, result, "mine", mine);
    set_key(scope, result, "theirs", theirs);
    Ok(result.into())
}

fn concat_arrays<'s>(
    scope: &mut v8::HandleScope<'s>,
    left: v8::Local<'s, v8::Value>,
    right: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let left = array(left).unwrap_or_else(|| empty_array(scope));
    let right = array(right).unwrap_or_else(|| empty_array(scope));
    let output = v8::Array::new(scope, (left.length() + right.length()) as i32);
    for index in 0..left.length() {
        let value = left
            .get_index(scope, index)
            .ok_or_else(|| PENDING.to_string())?;
        output
            .set_index(scope, index, value)
            .ok_or_else(|| PENDING.to_string())?;
    }
    for index in 0..right.length() {
        let value = right
            .get_index(scope, index)
            .ok_or_else(|| PENDING.to_string())?;
        output
            .set_index(scope, left.length() + index, value)
            .ok_or_else(|| PENDING.to_string())?;
    }
    Ok(output.into())
}

fn trade_confirm_ready<'s>(
    scope: &mut v8::HandleScope<'s>,
    facts: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let mine = [3542, 3538];
    let theirs = [3532, 3539];
    let mine_ready = mine
        .iter()
        .map(|id| component_items(scope, facts, *id))
        .collect::<Result<Vec<_>, _>>()?
        .iter()
        .any(|rows| array(*rows).is_some_and(|rows| rows.length() > 0))
        || widget_text_nonempty(scope, facts, 3557)?;
    let theirs_ready = theirs
        .iter()
        .map(|id| component_items(scope, facts, *id))
        .collect::<Result<Vec<_>, _>>()?
        .iter()
        .any(|rows| array(*rows).is_some_and(|rows| rows.length() > 0))
        || widget_text_nonempty(scope, facts, 3558)?;
    Ok(v8::Boolean::new(scope, mine_ready && theirs_ready).into())
}
fn widget_text_nonempty<'s>(
    scope: &mut v8::HandleScope<'s>,
    facts: v8::Local<'s, v8::Value>,
    component_id: i32,
) -> Result<bool, String> {
    let Some(row) = widget_by_id(scope, facts, component_id)? else {
        return Ok(false);
    };
    let text = property(scope, row, "text")?;
    Ok(text.is_string() && !text.to_rust_string_lossy(scope).is_empty())
}

fn duel_offers<'s>(
    scope: &mut v8::HandleScope<'s>,
    snapshot: v8::Local<'s, v8::Value>,
    facts: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let modal = object_i32(scope, snapshot, "main_modal_id")?.unwrap_or(-1);
    let ids = match modal {
        6575 => Some((6669, 6670)),
        6412 => Some((6507, 6508)),
        _ => None,
    };
    let (mine, theirs, ready) = if let Some((mine_id, their_id)) = ids {
        let mine = component_items(scope, facts, mine_id)?;
        let theirs = component_items(scope, facts, their_id)?;
        let ready = if modal == 6575 {
            let mine_ready = widget_by_id(scope, facts, mine_id)?
                .is_some_and(|row| bool_property(scope, row, "inventory_ready").unwrap_or(false));
            let theirs_ready = widget_by_id(scope, facts, their_id)?
                .is_some_and(|row| bool_property(scope, row, "inventory_ready").unwrap_or(false));
            mine_ready && theirs_ready
        } else {
            let mine_text = widget_by_id(scope, facts, 6516)?
                .map(|row| property(scope, row, "text"))
                .transpose()?
                .unwrap_or(v8::null(scope).into());
            let their_text = widget_by_id(scope, facts, 6517)?
                .map(|row| property(scope, row, "text"))
                .transpose()?
                .unwrap_or(v8::null(scope).into());
            trim_string(scope, mine_text) == "Absolutely nothing!"
                && trim_string(scope, their_text) == "Absolutely nothing!"
        };
        (mine, theirs, ready)
    } else {
        (empty_array(scope).into(), empty_array(scope).into(), false)
    };
    let result = v8::Object::new(scope);
    set_bool(scope, result, "ready", ready);
    set_key(scope, result, "mine", mine);
    set_key(scope, result, "theirs", theirs);
    Ok(result.into())
}

fn trim_string(scope: &mut v8::HandleScope<'_>, value: v8::Local<v8::Value>) -> String {
    if value.is_string() {
        value.to_rust_string_lossy(scope).trim().to_string()
    } else {
        String::new()
    }
}

fn main_skill_multi_items<'s>(
    scope: &mut v8::HandleScope<'s>,
    snapshot: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    item_rows_property(scope, snapshot, "main_make_items")
}

fn quest_statuses<'s>(
    scope: &mut v8::HandleScope<'s>,
    snapshot: v8::Local<'s, v8::Value>,
    facts: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let root = side_tab_interface_value(scope, snapshot, 2)?;
    if root < 0 {
        return Ok(empty_array(scope).into());
    }
    let rows = widget_rows_under(scope, facts, root)?;
    let mut result_rows = Vec::new();
    for row in rows {
        if object_i32(scope, row, "type_")? != Some(4) {
            continue;
        }
        let name = property(scope, row, "text")?;
        if !name.is_string() || name.to_rust_string_lossy(scope).is_empty() {
            continue;
        }
        result_rows.push((
            object_i32(scope, row, "component_id")?.unwrap_or(-1),
            name,
            object_i32(scope, row, "colour")?.unwrap_or(0),
        ));
    }
    let output = v8::Array::new(scope, result_rows.len() as i32);
    for (index, (component_id, name, colour)) in result_rows.into_iter().enumerate() {
        let row = v8::Object::new(scope);
        set_i32(scope, row, "comId", component_id);
        set_key(scope, row, "name", name);
        set_i32(scope, row, "colour", colour);
        output
            .set_index(scope, index as u32, row.into())
            .ok_or_else(|| PENDING.to_string())?;
    }
    Ok(output.into())
}

fn side_tab_interface_value<'s>(
    scope: &mut v8::HandleScope<'s>,
    snapshot: v8::Local<'s, v8::Value>,
    tab: i32,
) -> Result<i32, String> {
    let rows = property(scope, snapshot, "side_tab_ifaces")?;
    let Some(rows) = array(rows) else {
        return Ok(-1);
    };
    for index in 0..rows.length() {
        let row = rows
            .get_index(scope, index)
            .ok_or_else(|| PENDING.to_string())?;
        if object_i32(scope, row, "index")? == Some(tab) {
            return Ok(object_i32(scope, row, "id")?.unwrap_or(-1));
        }
    }
    Ok(-1)
}

fn make_products<'s>(
    scope: &mut v8::HandleScope<'s>,
    snapshot: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let source = property(scope, snapshot, "make_products")?;
    let Some(source) = array(source) else {
        return Ok(empty_array(scope).into());
    };
    let output = v8::Array::new(scope, source.length() as i32);
    for index in 0..source.length() {
        let product = source
            .get_index(scope, index)
            .ok_or_else(|| PENDING.to_string())?;
        let result = v8::Object::new(scope);
        let obj = property(scope, product, "object_id")?;
        let name = property(scope, product, "name")?;
        let buttons = property(scope, product, "buttons")?;
        let buttons = copy_make_buttons(scope, buttons)?;
        set_key(scope, result, "obj", obj);
        set_key(scope, result, "name", name);
        set_key(scope, result, "buttons", buttons);
        output
            .set_index(scope, index, result.into())
            .ok_or_else(|| PENDING.to_string())?;
    }
    Ok(output.into())
}

fn copy_make_buttons<'s>(
    scope: &mut v8::HandleScope<'s>,
    source: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let Some(source) = array(source) else {
        return Ok(empty_array(scope).into());
    };
    let output = v8::Array::new(scope, source.length() as i32);
    for index in 0..source.length() {
        let button = source
            .get_index(scope, index)
            .ok_or_else(|| PENDING.to_string())?;
        let result = v8::Object::new(scope);
        let qty = property(scope, button, "qty")?;
        let com_id = property(scope, button, "comId")?;
        set_key(scope, result, "qty", qty);
        set_key(scope, result, "comId", com_id);
        output
            .set_index(scope, index, result.into())
            .ok_or_else(|| PENDING.to_string())?;
    }
    Ok(output.into())
}

fn modal_close_observation<'s>(
    scope: &mut v8::HandleScope<'s>,
    snapshot: v8::Local<'s, v8::Value>,
    facts: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let attached = bool_property(scope, facts, "attached")?;
    let ingame = bool_property(scope, snapshot, "ingame")?;
    let session = property(scope, facts, "session_generation")?;
    let generation = property(scope, facts, "modal_close_generation")?;
    let session_number = session.number_value(scope).unwrap_or(f64::NAN);
    let generation_number = generation.number_value(scope).unwrap_or(f64::NAN);
    let safe = |value: f64| {
        value.is_finite() && value.fract() == 0.0 && value.abs() <= 9_007_199_254_740_991.0
    };
    let context = scope.get_current_context();
    let global = context.global(scope);
    let state_key = v8_str(scope, "__rs2b0t_client_adapter_modal_session")?;
    if !attached
        || !ingame
        || !safe(session_number)
        || !safe(generation_number)
        || session_number < 0.0
        || generation_number < 0.0
    {
        let null = v8::null(scope);
        global
            .set(scope, state_key, null.into())
            .ok_or_else(|| PENDING.to_string())?;
        return Ok(v8::null(scope).into());
    }
    let previous = global
        .get(scope, state_key)
        .ok_or_else(|| PENDING.to_string())?;
    let previous_session = property(scope, previous, "session_generation")?;
    let previous_session = previous_session.number_value(scope).unwrap_or(f64::NAN);
    let token = if previous.is_object() && previous_session == session_number {
        property(scope, previous, "token")?
    } else {
        let description =
            v8::String::new(scope, "client-adapter-session").ok_or_else(|| PENDING.to_string())?;
        v8::Symbol::new(scope, Some(description)).into()
    };
    let state = v8::Object::new(scope);
    set_number(scope, state, "session_generation", session_number);
    set_key(scope, state, "token", token);
    global
        .set(scope, state_key, state.into())
        .ok_or_else(|| PENDING.to_string())?;
    let result = v8::Object::new(scope);
    set_key(scope, result, "session", token);
    set_number(scope, result, "generation", generation_number);
    Ok(result.into())
}
struct ProjectionContext<'s> {
    level: i32,
    camera_x: i32,
    camera_y: i32,
    camera_z: i32,
    sin_pitch: i32,
    cos_pitch: i32,
    sin_yaw: i32,
    cos_yaw: i32,
    heights: v8::Local<'s, v8::Value>,
    heights_above: v8::Local<'s, v8::Value>,
    link_below: v8::Local<'s, v8::Value>,
    locs: v8::Local<'s, v8::Value>,
}

fn projection_context<'s>(
    scope: &mut v8::HandleScope<'s>,
    facts: v8::Local<'s, v8::Value>,
) -> Result<Option<ProjectionContext<'s>>, String> {
    if !bool_property(scope, facts, "attached")? {
        return Ok(None);
    }
    let scene = property(scope, facts, "projection_scene")?;
    if !scene.is_object() || scene.is_null() || scene.is_undefined() {
        return Ok(None);
    }
    Ok(Some(ProjectionContext {
        level: object_i32(scope, scene, "level")?.unwrap_or(0),
        camera_x: object_i32(scope, scene, "camera_x")?.unwrap_or(0),
        camera_y: object_i32(scope, scene, "camera_y")?.unwrap_or(0),
        camera_z: object_i32(scope, scene, "camera_z")?.unwrap_or(0),
        sin_pitch: object_i32(scope, scene, "sin_pitch")?.unwrap_or(0),
        cos_pitch: object_i32(scope, scene, "cos_pitch")?.unwrap_or(0),
        sin_yaw: object_i32(scope, scene, "sin_yaw")?.unwrap_or(0),
        cos_yaw: object_i32(scope, scene, "cos_yaw")?.unwrap_or(0),
        heights: property(scope, scene, "heights")?,
        heights_above: property(scope, scene, "heights_above")?,
        link_below: property(scope, scene, "link_below")?,
        locs: property(scope, scene, "locs")?,
    }))
}

fn world_tile_to_scene<'s>(
    scope: &mut v8::HandleScope<'s>,
    facts: v8::Local<'s, v8::Value>,
    x: v8::Local<'s, v8::Value>,
    z: v8::Local<'s, v8::Value>,
    u: v8::Local<'s, v8::Value>,
    v: v8::Local<'s, v8::Value>,
) -> Result<Option<(i32, i32)>, String> {
    let base = property(scope, facts, "map_build_base")?;
    let Some(base_x) = object_i32(scope, base, "x")? else {
        return Ok(None);
    };
    let Some(base_z) = object_i32(scope, base, "z")? else {
        return Ok(None);
    };
    let lx = js_number(scope, x)? - f64::from(base_x);
    let lz = js_number(scope, z)? - f64::from(base_z);
    if lx < 0.0 || lz < 0.0 || lx >= 104.0 || lz >= 104.0 {
        return Ok(None);
    }
    let u = if u.is_undefined() {
        0.5
    } else {
        js_number(scope, u)?
    };
    let v = if v.is_undefined() {
        0.5
    } else {
        js_number(scope, v)?
    };
    let scene_x = (lx as i32).wrapping_shl(7).wrapping_add(scene_fraction(u));
    let scene_z = (lz as i32).wrapping_shl(7).wrapping_add(scene_fraction(v));
    Ok(Some((scene_x, scene_z)))
}

fn js_number(scope: &mut v8::HandleScope, value: v8::Local<v8::Value>) -> Result<f64, String> {
    value.number_value(scope).ok_or_else(|| PENDING.to_string())
}

fn scene_fraction(value: f64) -> i32 {
    let clamped = if value.is_nan() {
        f64::NAN
    } else {
        value.clamp(0.0, 1.0)
    };
    (clamped * 128.0).floor().min(127.0) as i32
}

fn overlay_pos_world<'s>(
    scope: &mut v8::HandleScope<'s>,
    facts: v8::Local<'s, v8::Value>,
    arguments: [v8::Local<'s, v8::Value>; 5],
    area_game: bool,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let [x, z, height, u, v] = arguments;
    let Some(context) = projection_context(scope, facts)? else {
        return Ok(v8::null(scope).into());
    };
    let Some((scene_x, scene_z)) = world_tile_to_scene(scope, facts, x, z, u, v)? else {
        return Ok(v8::null(scope).into());
    };
    let height = js_i32(scope, height)?.unwrap_or(0);
    let Some((x, y)) = project_scene_point(scope, &context, scene_x, scene_z, height, area_game)?
    else {
        return Ok(v8::null(scope).into());
    };
    point_object(scope, x, y)
}

fn project_scene_point<'s>(
    scope: &mut v8::HandleScope<'s>,
    context: &ProjectionContext<'s>,
    x: i32,
    z: i32,
    height: i32,
    area_game: bool,
) -> Result<Option<(i32, i32)>, String> {
    if !(128..=13_056).contains(&x) || !(128..=13_056).contains(&z) {
        return Ok(None);
    }
    let Some(ground_y) = projection_height_at(scope, context, x, z)? else {
        return Ok(None);
    };
    let y = ground_y.wrapping_sub(height);
    let dx = x.wrapping_sub(context.camera_x);
    let dy = y.wrapping_sub(context.camera_y);
    let dz = z.wrapping_sub(context.camera_z);
    let projected_x = dz
        .wrapping_mul(context.sin_yaw)
        .wrapping_add(dx.wrapping_mul(context.cos_yaw))
        >> 16;
    let depth_yaw = dz
        .wrapping_mul(context.cos_yaw)
        .wrapping_sub(dx.wrapping_mul(context.sin_yaw))
        >> 16;
    let projected_y = dy
        .wrapping_mul(context.cos_pitch)
        .wrapping_sub(depth_yaw.wrapping_mul(context.sin_pitch))
        >> 16;
    let depth = dy
        .wrapping_mul(context.sin_pitch)
        .wrapping_add(depth_yaw.wrapping_mul(context.cos_pitch))
        >> 16;
    if depth < 50 {
        return Ok(None);
    }
    let origin_offset = if area_game { 0 } else { 4 };
    let px = 256i32
        .wrapping_add(origin_offset)
        .wrapping_add(projected_x.wrapping_shl(9) / depth);
    let py = 167i32
        .wrapping_add(origin_offset)
        .wrapping_add(projected_y.wrapping_shl(9) / depth);
    Ok(Some((px, py)))
}

fn projection_height_at<'s>(
    scope: &mut v8::HandleScope<'s>,
    context: &ProjectionContext<'s>,
    x: i32,
    z: i32,
) -> Result<Option<i32>, String> {
    if !(0..=103).contains(&(x >> 7)) || !(0..=103).contains(&(z >> 7)) {
        return Ok(Some(0));
    }
    if !(0..=3).contains(&context.level) {
        return Ok(None);
    }
    let tile_x = x >> 7;
    let tile_z = z >> 7;
    let link_index = (tile_x * 104 + tile_z) as u32;
    let linked = context.level < 3
        && integer_at(scope, context.link_below, link_index)?.unwrap_or(0) & 1 != 0;
    let heights = if linked {
        context.heights_above
    } else {
        context.heights
    };
    let tile_local_x = x & 0x7f;
    let tile_local_z = z & 0x7f;
    let base = (tile_x * 105 + tile_z) as u32;
    let Some(h00) = integer_at(scope, heights, base)? else {
        return Ok(None);
    };
    let Some(h10) = integer_at(scope, heights, base + 105)? else {
        return Ok(None);
    };
    let Some(h01) = integer_at(scope, heights, base + 1)? else {
        return Ok(None);
    };
    let Some(h11) = integer_at(scope, heights, base + 106)? else {
        return Ok(None);
    };
    let y00 = (h00
        .wrapping_mul(128 - tile_local_x)
        .wrapping_add(h10.wrapping_mul(tile_local_x)))
        >> 7;
    let y11 = (h01
        .wrapping_mul(128 - tile_local_x)
        .wrapping_add(h11.wrapping_mul(tile_local_x)))
        >> 7;
    Ok(Some(
        y00.wrapping_mul(128 - tile_local_z)
            .wrapping_add(y11.wrapping_mul(tile_local_z))
            >> 7,
    ))
}

fn integer_at<'s>(
    scope: &mut v8::HandleScope<'s>,
    source: v8::Local<'s, v8::Value>,
    index: u32,
) -> Result<Option<i32>, String> {
    let value = indexed_value(scope, source, index)?;
    js_i32(scope, value)
}

fn indexed_value<'s>(
    scope: &mut v8::HandleScope<'s>,
    source: v8::Local<'s, v8::Value>,
    index: u32,
) -> Result<v8::Local<'s, v8::Value>, String> {
    if let Some(source) = array(source) {
        return source
            .get_index(scope, index)
            .ok_or_else(|| PENDING.to_string());
    }
    if source.is_array_buffer_view() {
        let source = source.to_object(scope).ok_or_else(|| PENDING.to_string())?;
        return source
            .get_index(scope, index)
            .ok_or_else(|| PENDING.to_string());
    }
    Ok(v8::undefined(scope).into())
}

fn point_object<'s>(
    scope: &mut v8::HandleScope<'s>,
    x: i32,
    y: i32,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let object = v8::Object::new(scope);
    set_i32(scope, object, "x", x);
    set_i32(scope, object, "y", y);
    Ok(object.into())
}

fn loc_box<'s>(
    scope: &mut v8::HandleScope<'s>,
    _snapshot: v8::Local<'s, v8::Value>,
    facts: v8::Local<'s, v8::Value>,
    options: v8::Local<'s, v8::Value>,
    area_game: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let Some(context) = projection_context(scope, facts)? else {
        return Ok(v8::null(scope).into());
    };
    if !options.is_object() || options.is_null() {
        return Ok(v8::null(scope).into());
    }
    let Some(x) = object_i32(scope, options, "x")? else {
        return Ok(v8::null(scope).into());
    };
    let Some(z) = object_i32(scope, options, "z")? else {
        return Ok(v8::null(scope).into());
    };
    let base = property(scope, facts, "map_build_base")?;
    let Some(base_x) = object_i32(scope, base, "x")? else {
        return Ok(v8::null(scope).into());
    };
    let Some(base_z) = object_i32(scope, base, "z")? else {
        return Ok(v8::null(scope).into());
    };
    let lx = x.saturating_sub(base_x);
    let lz = z.saturating_sub(base_z);
    if !(0..104).contains(&lx) || !(0..104).contains(&lz) {
        return Ok(v8::null(scope).into());
    }

    let name = property(scope, options, "name")?;
    let wanted_name = if name.is_string() {
        let name = name.to_rust_string_lossy(scope);
        let name = name.trim().to_lowercase();
        (!name.is_empty()).then_some(name)
    } else {
        None
    };
    let id_value = property(scope, options, "id")?;
    let has_id = !id_value.is_undefined();
    let wanted_id = js_i32(scope, id_value)?;
    let level_value = property(scope, options, "level")?;
    let level = if level_value.is_null() || level_value.is_undefined() {
        context.level
    } else {
        js_i32(scope, level_value)?.unwrap_or(context.level)
    };
    let has_selector = has_id || wanted_name.is_some();
    let search = if has_selector { 2 } else { 0 };
    let locs = array(context.locs).unwrap_or_else(|| empty_array(scope));
    let mut best = None;
    for index in 0..locs.length() {
        let loc = locs
            .get_index(scope, index)
            .ok_or_else(|| PENDING.to_string())?;
        if object_i32(scope, loc, "level")? != Some(level) {
            continue;
        }
        let id = object_i32(scope, loc, "id")?.unwrap_or(-1);
        if has_id {
            if wanted_id != Some(id) {
                continue;
            }
        } else if let Some(wanted) = wanted_name.as_deref() {
            let name = property(scope, loc, "name")?;
            if !name.is_string()
                || name.to_rust_string_lossy(scope).to_lowercase().as_str() != wanted
            {
                continue;
            }
        }
        let min_x = object_i32(scope, loc, "min_tile_x")?.unwrap_or(-1);
        let min_z = object_i32(scope, loc, "min_tile_z")?.unwrap_or(-1);
        let max_x = object_i32(scope, loc, "max_tile_x")?.unwrap_or(-1);
        let max_z = object_i32(scope, loc, "max_tile_z")?.unwrap_or(-1);
        let rank = object_i32(scope, loc, "rank")?.unwrap_or(i32::MAX);
        if (min_x - lx).abs() > search || (min_z - lz).abs() > search {
            continue;
        }
        if rank == 1 && (lx < min_x || lx > max_x || lz < min_z || lz > max_z) {
            continue;
        }
        let dist = (min_x - lx).abs().saturating_add((min_z - lz).abs());
        let candidate = LocBoxCandidate {
            rank,
            dist,
            scene_x: object_i32(scope, loc, "scene_x")?.unwrap_or(0),
            scene_z: object_i32(scope, loc, "scene_z")?.unwrap_or(0),
            half_width: object_i32(scope, loc, "half_width")?.unwrap_or(64),
            half_length: object_i32(scope, loc, "half_length")?.unwrap_or(64),
            top_height: object_i32(scope, loc, "top_height")?.unwrap_or(128),
        };
        if best.as_ref().is_none_or(|best: &LocBoxCandidate| {
            candidate.rank < best.rank
                || (candidate.rank == best.rank && candidate.dist < best.dist)
        }) {
            best = Some(candidate);
        }
    }
    let Some(best) = best else {
        return Ok(v8::null(scope).into());
    };
    let area_game = area_game.boolean_value(scope);
    loc_box_points(scope, &context, best, area_game)
}

#[derive(Clone, Copy)]
struct LocBoxCandidate {
    rank: i32,
    dist: i32,
    scene_x: i32,
    scene_z: i32,
    half_width: i32,
    half_length: i32,
    top_height: i32,
}

fn loc_box_points<'s>(
    scope: &mut v8::HandleScope<'s>,
    context: &ProjectionContext<'s>,
    loc: LocBoxCandidate,
    area_game: bool,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let half_width = loc.half_width.max(32);
    let half_length = loc.half_length.max(32);
    let corners = [
        (-half_width, -half_length),
        (half_width, -half_length),
        (half_width, half_length),
        (-half_width, half_length),
    ];
    let top_height = loc.top_height;
    let try_heights = [top_height.max(16), top_height.clamp(16, 96), 48];
    for top in try_heights {
        let output = v8::Array::new(scope, 8);
        let mut output_index = 0;
        let mut complete = true;
        for height in [4, top] {
            for (dx, dz) in corners {
                let x = loc.scene_x.saturating_add(dx);
                let z = loc.scene_z.saturating_add(dz);
                let Some((x, y)) = project_scene_point(scope, context, x, z, height, area_game)?
                else {
                    complete = false;
                    break;
                };
                let point = point_object(scope, x, y)?;
                output
                    .set_index(scope, output_index, point)
                    .ok_or_else(|| PENDING.to_string())?;
                output_index += 1;
            }
            if !complete {
                break;
            }
        }
        if complete {
            return Ok(output.into());
        }
    }
    Ok(v8::null(scope).into())
}

fn item_icon_pixels<'s>(
    scope: &mut v8::HandleScope<'s>,
    facts: v8::Local<'s, v8::Value>,
    id: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    if !bool_property(scope, facts, "attached")? {
        return Ok(v8::null(scope).into());
    }
    let Some(id) = js_i32(scope, id)? else {
        return Ok(v8::null(scope).into());
    };
    let Some(pixels) =
        api::snapshot::client_adapter_cache().and_then(|cache| cache.item_icon_pixels(id))
    else {
        return Ok(v8::null(scope).into());
    };
    let width = pixels.width();
    let height = pixels.height();
    let values = pixels.data();
    let byte_len = values
        .len()
        .checked_mul(std::mem::size_of::<i32>())
        .ok_or_else(|| "item icon byte length".to_string())?;
    let buffer = v8::ArrayBuffer::new(scope, byte_len);
    if !values.is_empty() {
        let backing = buffer.get_backing_store();
        let Some(ptr) = backing.data() else {
            return Err("item icon backing store".to_string());
        };
        let dest = ptr.as_ptr() as *mut i32;
        for (index, value) in values.iter().enumerate() {
            // SAFETY: V8 allocated `byte_len` bytes and `values` supplies
            // exactly the corresponding number of native-endian i32 pixels.
            unsafe { dest.add(index).write_unaligned(*value) };
        }
    }
    let data = v8::Int32Array::new(scope, buffer, 0, values.len())
        .ok_or_else(|| "item icon Int32Array".to_string())?;
    let result = v8::Object::new(scope);
    set_i32(scope, result, "width", width);
    set_i32(scope, result, "height", height);
    set_key(scope, result, "data", data.into());
    Ok(result.into())
}

#[cfg(test)]
#[path = "client_adapter_v8_tests.rs"]
mod tests;
