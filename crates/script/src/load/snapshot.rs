//! FlatBuffer-to-V8 snapshot materialization and helpers (feature `load` only).
//!
//! Builds `__rs2b0t_host.snapshot` on the isolate thread from decoded
//! FlatBuffers (delta merge, absent-vs-empty, object reuse). Also owns the
//! small V8 property helpers and native-event batch materialization used by
//! the tick loop. Lifecycle/teardown stays in `isolate`.

use rustyscript::Runtime;

/// Materialise the decoded FlatBuffer snapshot as the JS object the
/// shim reads (`__rs2b0t_host.snapshot`), merging it onto the last
/// posted object. A post is a delta: `tick` is always carried, other
/// fields only when they changed — so an omitted vector must NOT clear
/// the previous JS rows. Only the fields the buffer carries are
/// overwritten; the first post (keyframe on Start / isolate spawn)
/// builds the object and fail-closes the fields the keyframe also
/// lacks (absent `here`, empty rows, false flags), exactly like the
/// old JSON blob. The object is built on the isolate thread directly
/// from the buffer (v8 object construction — not `JSON.parse`): a wall
/// of 50+ isolates never parses a JSON document per tick.
pub(super) fn materialize_snapshot(
    runtime: &mut Runtime,
    snap: &crate::isolate_fb::SnapshotReader<'_>,
    host_hold: bool,
) -> Result<(), String> {
    let context = runtime.deno_runtime().main_context();
    let mut scope = runtime.deno_runtime().handle_scope();
    let global = context.open(&mut scope).global(&mut scope);
    let host_key = js_string(&mut scope, "__rs2b0t_host")?;
    let host = global
        .get(&mut scope, host_key)
        .ok_or_else(|| "no __rs2b0t_host global".to_string())?
        .to_object(&mut scope)
        .ok_or_else(|| "__rs2b0t_host is not an object".to_string())?;

    let snap_key = js_string(&mut scope, "snapshot")?;
    let existing = host.get(&mut scope, snap_key);
    let had = existing.is_some_and(|v| v.is_object());
    let obj = if had {
        existing
            .expect("checked above")
            .to_object(&mut scope)
            .ok_or_else(|| "snapshot is not an object".to_string())?
    } else {
        v8::Object::new(&mut scope)
    };
    // The fail-closed defaults a keyframe's absent fields materialise
    // to (the same values the shim's `snap()` reads with no snapshot).
    let empty_rows: v8::Local<v8::Value> = v8::Array::new(&mut scope, 0).into();
    let none: v8::Local<v8::Value> = v8::null(&mut scope).into();

    // `tick` is always carried. A field the buffer carries overwrites
    // the object; a field a delta omits keeps its last value. On the
    // keyframe (`had` is false) an absent field fail-closes to the
    // same value the shim's `snap()` reads without a snapshot.
    let tick = num(&mut scope, snap.tick() as f64);
    set(&mut scope, obj, "tick", tick)?;
    let falsy: v8::Local<v8::Value> = v8::Boolean::new(&mut scope, false).into();
    if snap.has_here() {
        let here = match snap.here() {
            Some(tile) => tile_object(&mut scope, &tile)?,
            None => v8::null(&mut scope).into(),
        };
        set(&mut scope, obj, "here", here)?;
        // The reader adapter's `worldTile` reads the host handle
        // directly (not the snapshot blob): mirror `here` there.
        set(&mut scope, host, "tile", here)?;
    } else if !had {
        set(&mut scope, obj, "here", none)?;
    }
    // The reader adapter's `inventorySize` reads the host handle too:
    // mirror the inv tab slot count (0 while the inv tab is
    // tutorial-locked — the gate an onStart waits on).
    if snap.has_inv_size() {
        let inv_size = num(&mut scope, snap.inv_size() as f64);
        set(&mut scope, host, "invSize", inv_size)?;
        set(&mut scope, obj, "inv_size", inv_size)?;
    }
    if snap.has_ingame() {
        let ingame = v8::Boolean::new(&mut scope, snap.ingame());
        set(&mut scope, obj, "ingame", ingame.into())?;
    } else if !had {
        set(&mut scope, obj, "ingame", falsy)?;
    }
    if snap.has_inv() {
        let inv = row_array(&mut scope, &snap.inv())?;
        set(&mut scope, obj, "inv", inv)?;
    } else if !had {
        set(&mut scope, obj, "inv", empty_rows)?;
    }
    if snap.has_stats() {
        let stats = stat_array(&mut scope, &snap.stats())?;
        set(&mut scope, obj, "stats", stats)?;
    } else if !had {
        set(&mut scope, obj, "stats", empty_rows)?;
    }
    if snap.has_booths() {
        let booths = tile_array(&mut scope, &snap.booths())?;
        set(&mut scope, obj, "booths", booths)?;
    } else if !had {
        set(&mut scope, obj, "booths", empty_rows)?;
    }
    if snap.has_nearest_booth() {
        let nb = nearest_booth_object(&mut scope, &snap.nearest_booth().expect("has flag"))?;
        set(&mut scope, obj, "nearest_booth", nb)?;
    } else if !had {
        set(&mut scope, obj, "nearest_booth", none)?;
    }
    if snap.has_banks() {
        let banks = bank_stand_array(&mut scope, &snap.banks())?;
        set(&mut scope, obj, "banks", banks)?;
    } else if !had {
        set(&mut scope, obj, "banks", empty_rows)?;
    }
    if snap.has_bank() {
        let bank = row_array(&mut scope, &snap.bank())?;
        set(&mut scope, obj, "bank", bank)?;
    } else if !had {
        set(&mut scope, obj, "bank", empty_rows)?;
    }
    if snap.has_bank_side() {
        let bank_side = row_array(&mut scope, &snap.bank_side())?;
        set(&mut scope, obj, "bank_side", bank_side)?;
    } else if !had {
        set(&mut scope, obj, "bank_side", empty_rows)?;
    }
    if snap.has_bank_open() {
        let bank_open = v8::Boolean::new(&mut scope, snap.bank_open());
        set(&mut scope, obj, "bank_open", bank_open.into())?;
    } else if !had {
        set(&mut scope, obj, "bank_open", falsy)?;
    }
    if snap.has_bank_loaded() {
        let bank_loaded = v8::Boolean::new(&mut scope, snap.bank_loaded());
        set(&mut scope, obj, "bank_loaded", bank_loaded.into())?;
    } else if !had {
        set(&mut scope, obj, "bank_loaded", falsy)?;
    }
    if snap.has_bank_generation() {
        let bank_generation = num(&mut scope, snap.bank_generation() as f64);
        set(&mut scope, obj, "bank_generation", bank_generation)?;
    } else if !had {
        let bank_generation = num(&mut scope, 0.0);
        set(&mut scope, obj, "bank_generation", bank_generation)?;
    }
    if snap.has_bank_approaches() {
        let approaches = bank_approach_array(&mut scope, &snap.bank_approaches())?;
        set(&mut scope, obj, "bank_approaches", approaches)?;
    } else if !had {
        set(&mut scope, obj, "bank_approaches", empty_rows)?;
    }
    if snap.has_walk_outcome_seq() {
        let seq = num(&mut scope, snap.walk_outcome_seq() as f64);
        set(&mut scope, obj, "walk_outcome_seq", seq)?;
        let gen = num(&mut scope, snap.walk_outcome_generation() as f64);
        set(&mut scope, obj, "walk_outcome_generation", gen)?;
        let failed = v8::Boolean::new(&mut scope, snap.walk_outcome_failed());
        set(&mut scope, obj, "walk_outcome_failed", failed.into())?;
        let x = num(&mut scope, snap.walk_outcome_x() as f64);
        set(&mut scope, obj, "walk_outcome_x", x)?;
        let z = num(&mut scope, snap.walk_outcome_z() as f64);
        set(&mut scope, obj, "walk_outcome_z", z)?;
        let level = num(&mut scope, snap.walk_outcome_level() as f64);
        set(&mut scope, obj, "walk_outcome_level", level)?;
        let radius = num(&mut scope, snap.walk_outcome_radius() as f64);
        set(&mut scope, obj, "walk_outcome_radius", radius)?;
        let allow = v8::Boolean::new(&mut scope, snap.walk_outcome_allow_teleports());
        set(&mut scope, obj, "walk_outcome_allow_teleports", allow.into())?;
        let rid = num(&mut scope, snap.walk_outcome_request_id() as f64);
        set(&mut scope, obj, "walk_outcome_request_id", rid)?;
    } else if !had {
        let zero = num(&mut scope, 0.0);
        set(&mut scope, obj, "walk_outcome_seq", zero)?;
        set(&mut scope, obj, "walk_outcome_generation", zero)?;
        set(&mut scope, obj, "walk_outcome_failed", falsy)?;
        set(&mut scope, obj, "walk_outcome_x", zero)?;
        set(&mut scope, obj, "walk_outcome_z", zero)?;
        set(&mut scope, obj, "walk_outcome_level", zero)?;
        set(&mut scope, obj, "walk_outcome_radius", zero)?;
        set(&mut scope, obj, "walk_outcome_allow_teleports", falsy)?;
        set(&mut scope, obj, "walk_outcome_request_id", zero)?;
    }
    if snap.has_route_inspect_seq() {
        let seq = num(&mut scope, snap.route_inspect_seq() as f64);
        set(&mut scope, obj, "route_inspect_seq", seq)?;
        let gen = num(&mut scope, snap.route_inspect_generation() as f64);
        set(&mut scope, obj, "route_inspect_generation", gen)?;
        let rid = num(&mut scope, snap.route_inspect_request_id() as f64);
        set(&mut scope, obj, "route_inspect_request_id", rid)?;
        let ok = v8::Boolean::new(&mut scope, snap.route_inspect_ok());
        set(&mut scope, obj, "route_inspect_ok", ok.into())?;
        let reason = js_string(&mut scope, snap.route_inspect_reason())?;
        set(&mut scope, obj, "route_inspect_reason", reason)?;
        let bank = v8::Boolean::new(&mut scope, snap.route_inspect_bank_planned());
        set(&mut scope, obj, "route_inspect_bank_planned", bank.into())?;
        let ticks = num(&mut scope, snap.route_inspect_ticks());
        set(&mut scope, obj, "route_inspect_ticks", ticks)?;
        let hops = inspect_hop_array(&mut scope, &snap.route_inspect_hops())?;
        set(&mut scope, obj, "route_inspect_hops", hops)?;
        let pseq = num(&mut scope, snap.route_inspect_prev_seq() as f64);
        set(&mut scope, obj, "route_inspect_prev_seq", pseq)?;
        let pgen = num(&mut scope, snap.route_inspect_prev_generation() as f64);
        set(&mut scope, obj, "route_inspect_prev_generation", pgen)?;
        let prid = num(&mut scope, snap.route_inspect_prev_request_id() as f64);
        set(&mut scope, obj, "route_inspect_prev_request_id", prid)?;
        let pok = v8::Boolean::new(&mut scope, snap.route_inspect_prev_ok());
        set(&mut scope, obj, "route_inspect_prev_ok", pok.into())?;
        let preason = js_string(&mut scope, snap.route_inspect_prev_reason())?;
        set(&mut scope, obj, "route_inspect_prev_reason", preason)?;
        let pbank = v8::Boolean::new(&mut scope, snap.route_inspect_prev_bank_planned());
        set(&mut scope, obj, "route_inspect_prev_bank_planned", pbank.into())?;
        let pticks = num(&mut scope, snap.route_inspect_prev_ticks());
        set(&mut scope, obj, "route_inspect_prev_ticks", pticks)?;
        let phops = inspect_hop_array(&mut scope, &snap.route_inspect_prev_hops())?;
        set(&mut scope, obj, "route_inspect_prev_hops", phops)?;
        let run = num(&mut scope, snap.route_inspect_running_id() as f64);
        set(&mut scope, obj, "route_inspect_running_id", run)?;
        let pend = num(&mut scope, snap.route_inspect_pending_id() as f64);
        set(&mut scope, obj, "route_inspect_pending_id", pend)?;
        let acc = num(&mut scope, snap.route_inspect_accepted_id() as f64);
        set(&mut scope, obj, "route_inspect_accepted_id", acc)?;
        let repl = num(&mut scope, snap.route_inspect_replaced_id() as f64);
        set(&mut scope, obj, "route_inspect_replaced_id", repl)?;
        let rprev = num(&mut scope, snap.route_inspect_replaced_prev_id() as f64);
        set(&mut scope, obj, "route_inspect_replaced_prev_id", rprev)?;
        let ref1 = num(&mut scope, snap.route_inspect_refused_id() as f64);
        set(&mut scope, obj, "route_inspect_refused_id", ref1)?;
        let ref2 = num(&mut scope, snap.route_inspect_refused_id_2() as f64);
        set(&mut scope, obj, "route_inspect_refused_id_2", ref2)?;
        let ref3 = num(&mut scope, snap.route_inspect_refused_id_3() as f64);
        set(&mut scope, obj, "route_inspect_refused_id_3", ref3)?;
        let unobs = num(&mut scope, snap.route_inspect_unobserved() as f64);
        set(&mut scope, obj, "route_inspect_unobserved", unobs)?;
    } else if !had {
        let zero = num(&mut scope, 0.0);
        set(&mut scope, obj, "route_inspect_seq", zero)?;
        set(&mut scope, obj, "route_inspect_generation", zero)?;
        set(&mut scope, obj, "route_inspect_request_id", zero)?;
        set(&mut scope, obj, "route_inspect_ok", falsy)?;
        let empty_reason = js_string(&mut scope, "")?;
        set(&mut scope, obj, "route_inspect_reason", empty_reason)?;
        set(&mut scope, obj, "route_inspect_bank_planned", falsy)?;
        set(&mut scope, obj, "route_inspect_ticks", zero)?;
        set(&mut scope, obj, "route_inspect_hops", empty_rows)?;
        set(&mut scope, obj, "route_inspect_prev_seq", zero)?;
        set(&mut scope, obj, "route_inspect_prev_generation", zero)?;
        set(&mut scope, obj, "route_inspect_prev_request_id", zero)?;
        set(&mut scope, obj, "route_inspect_prev_ok", falsy)?;
        let empty_prev_reason = js_string(&mut scope, "")?;
        set(&mut scope, obj, "route_inspect_prev_reason", empty_prev_reason)?;
        set(&mut scope, obj, "route_inspect_prev_bank_planned", falsy)?;
        set(&mut scope, obj, "route_inspect_prev_ticks", zero)?;
        set(&mut scope, obj, "route_inspect_prev_hops", empty_rows)?;
        set(&mut scope, obj, "route_inspect_running_id", zero)?;
        set(&mut scope, obj, "route_inspect_pending_id", zero)?;
        set(&mut scope, obj, "route_inspect_accepted_id", zero)?;
        set(&mut scope, obj, "route_inspect_replaced_id", zero)?;
        set(&mut scope, obj, "route_inspect_replaced_prev_id", zero)?;
        set(&mut scope, obj, "route_inspect_refused_id", zero)?;
        set(&mut scope, obj, "route_inspect_refused_id_2", zero)?;
        set(&mut scope, obj, "route_inspect_refused_id_3", zero)?;
        set(&mut scope, obj, "route_inspect_unobserved", zero)?;
    }
    if snap.has_count_dialog_open() {
        let count_dialog_open = v8::Boolean::new(&mut scope, snap.count_dialog_open());
        set(
            &mut scope,
            obj,
            "count_dialog_open",
            count_dialog_open.into(),
        )?;
    } else if !had {
        set(&mut scope, obj, "count_dialog_open", falsy)?;
    }
    if snap.has_withdraw_x_result_seq() {
        let seq = num(&mut scope, snap.withdraw_x_result_seq() as f64);
        set(&mut scope, obj, "withdraw_x_result_seq", seq)?;
    } else if !had {
        let seq = num(&mut scope, 0.0);
        set(&mut scope, obj, "withdraw_x_result_seq", seq)?;
    }
    if snap.has_withdraw_x_result() {
        let result = v8::Boolean::new(&mut scope, snap.withdraw_x_result());
        set(&mut scope, obj, "withdraw_x_result", result.into())?;
    } else if !had {
        set(&mut scope, obj, "withdraw_x_result", falsy)?;
    }
    if snap.has_withdraw_load_result_seq() {
        let seq = num(&mut scope, snap.withdraw_load_result_seq() as f64);
        set(&mut scope, obj, "withdraw_load_result_seq", seq)?;
    } else if !had {
        let seq = num(&mut scope, 0.0);
        set(&mut scope, obj, "withdraw_load_result_seq", seq)?;
    }
    if snap.has_withdraw_load_result() {
        let result = v8::Boolean::new(&mut scope, snap.withdraw_load_result());
        set(&mut scope, obj, "withdraw_load_result", result.into())?;
    } else if !had {
        set(&mut scope, obj, "withdraw_load_result", falsy)?;
    }
    if snap.has_bank_op_result_seq() {
        let seq = num(&mut scope, snap.bank_op_result_seq() as f64);
        set(&mut scope, obj, "bank_op_result_seq", seq)?;
    } else if !had {
        let seq = num(&mut scope, 0.0);
        set(&mut scope, obj, "bank_op_result_seq", seq)?;
    }
    if snap.has_bank_op_result() {
        let result = v8::Boolean::new(&mut scope, snap.bank_op_result());
        set(&mut scope, obj, "bank_op_result", result.into())?;
    } else if !had {
        set(&mut scope, obj, "bank_op_result", falsy)?;
    }
    if snap.has_bank_note_on() {
        let bank_note_on = num(&mut scope, snap.bank_note_on() as f64);
        set(&mut scope, obj, "bank_note_on", bank_note_on)?;
    } else if !had {
        let bank_note_on = num(&mut scope, -1.0);
        set(&mut scope, obj, "bank_note_on", bank_note_on)?;
    }
    if snap.has_bank_note_off() {
        let bank_note_off = num(&mut scope, snap.bank_note_off() as f64);
        set(&mut scope, obj, "bank_note_off", bank_note_off)?;
    } else if !had {
        let bank_note_off = num(&mut scope, -1.0);
        set(&mut scope, obj, "bank_note_off", bank_note_off)?;
    }
    if snap.has_scene_state() {
        let scene_state = num(&mut scope, snap.scene_state() as f64);
        set(&mut scope, obj, "scene_state", scene_state)?;
    } else if !had {
        let zero = num(&mut scope, 0.0);
        set(&mut scope, obj, "scene_state", zero)?;
    }
    if snap.has_weight() {
        let weight = num(&mut scope, snap.weight() as f64);
        set(&mut scope, obj, "weight", weight)?;
    } else if !had {
        let zero = num(&mut scope, 0.0);
        set(&mut scope, obj, "weight", zero)?;
    }
    if snap.has_combat_level() {
        let combat_level = num(&mut scope, snap.combat_level() as f64);
        set(&mut scope, obj, "combat_level", combat_level)?;
    } else if !had {
        let zero = num(&mut scope, 0.0);
        set(&mut scope, obj, "combat_level", zero)?;
    }
    if snap.has_camera_yaw() {
        let camera_yaw = num(&mut scope, snap.camera_yaw() as f64);
        set(&mut scope, obj, "camera_yaw", camera_yaw)?;
    } else if !had {
        let zero = num(&mut scope, 0.0);
        set(&mut scope, obj, "camera_yaw", zero)?;
    }
    if snap.has_camera_pitch() {
        let camera_pitch = num(&mut scope, snap.camera_pitch() as f64);
        set(&mut scope, obj, "camera_pitch", camera_pitch)?;
    } else if !had {
        let zero = num(&mut scope, 0.0);
        set(&mut scope, obj, "camera_pitch", zero)?;
    }
    if snap.has_teleports_enabled() {
        let teleports_enabled = v8::Boolean::new(&mut scope, snap.teleports_enabled());
        set(
            &mut scope,
            obj,
            "teleports_enabled",
            teleports_enabled.into(),
        )?;
    } else if !had {
        set(&mut scope, obj, "teleports_enabled", falsy)?;
    }
    if snap.has_self_slot() {
        let self_slot = num(&mut scope, snap.self_slot() as f64);
        set(&mut scope, obj, "self_slot", self_slot)?;
    } else if !had {
        let zero = num(&mut scope, 0.0);
        set(&mut scope, obj, "self_slot", zero)?;
    }
    if snap.has_trade_offer_open() {
        let trade_offer_open = v8::Boolean::new(&mut scope, snap.trade_offer_open());
        set(&mut scope, obj, "trade_offer_open", trade_offer_open.into())?;
    } else if !had {
        set(&mut scope, obj, "trade_offer_open", falsy)?;
    }
    if snap.has_trade_confirm_open() {
        let trade_confirm_open = v8::Boolean::new(&mut scope, snap.trade_confirm_open());
        set(
            &mut scope,
            obj,
            "trade_confirm_open",
            trade_confirm_open.into(),
        )?;
    } else if !had {
        set(&mut scope, obj, "trade_confirm_open", falsy)?;
    }
    if snap.has_trade_partner() {
        match snap.trade_partner() {
            Some(name) => {
                let partner = js_string(&mut scope, name)?;
                set(&mut scope, obj, "trade_partner", partner)?;
            }
            None => {
                let none = v8::null(&mut scope);
                set(&mut scope, obj, "trade_partner", none.into())?;
            }
        }
    } else if !had {
        let none = v8::null(&mut scope);
        set(&mut scope, obj, "trade_partner", none.into())?;
    }
    if snap.has_trade_mine() {
        let trade_mine = row_array(&mut scope, &snap.trade_mine())?;
        set(&mut scope, obj, "trade_mine", trade_mine)?;
    } else if !had {
        set(&mut scope, obj, "trade_mine", empty_rows)?;
    }
    if snap.has_trade_theirs() {
        let trade_theirs = row_array(&mut scope, &snap.trade_theirs())?;
        set(&mut scope, obj, "trade_theirs", trade_theirs)?;
    } else if !had {
        set(&mut scope, obj, "trade_theirs", empty_rows)?;
    }
    if snap.has_trade_side() {
        let trade_side = row_array(&mut scope, &snap.trade_side())?;
        set(&mut scope, obj, "trade_side", trade_side)?;
    } else if !had {
        set(&mut scope, obj, "trade_side", empty_rows)?;
    }
    if snap.has_trade_accept_id() {
        let trade_accept_id = num(&mut scope, snap.trade_accept_id() as f64);
        set(&mut scope, obj, "trade_accept_id", trade_accept_id)?;
    } else if !had {
        let trade_accept_id = num(&mut scope, -1.0);
        set(&mut scope, obj, "trade_accept_id", trade_accept_id)?;
    }
    if snap.has_trade_decline_id() {
        let trade_decline_id = num(&mut scope, snap.trade_decline_id() as f64);
        set(&mut scope, obj, "trade_decline_id", trade_decline_id)?;
    } else if !had {
        let trade_decline_id = num(&mut scope, -1.0);
        set(&mut scope, obj, "trade_decline_id", trade_decline_id)?;
    }
    if snap.has_shop_open() {
        let shop_open = v8::Boolean::new(&mut scope, snap.shop_open());
        set(&mut scope, obj, "shop_open", shop_open.into())?;
    } else if !had {
        set(&mut scope, obj, "shop_open", falsy)?;
    }
    if snap.has_shop_stock() {
        let shop_stock = row_array(&mut scope, &snap.shop_stock())?;
        set(&mut scope, obj, "shop_stock", shop_stock)?;
    } else if !had {
        set(&mut scope, obj, "shop_stock", empty_rows)?;
    }
    if snap.has_shop_player_available() {
        let shop_player = if snap.shop_player_available() {
            row_array(&mut scope, &snap.shop_player())?
        } else {
            empty_rows
        };
        set(&mut scope, obj, "shop_player", shop_player)?;
    } else if !had {
        set(&mut scope, obj, "shop_player", empty_rows)?;
    }
    if snap.has_main_make_available() {
        let available = v8::Boolean::new(&mut scope, snap.main_make_available());
        set(&mut scope, obj, "main_make_available", available.into())?;
        let main_make = if snap.main_make_available() {
            row_array(&mut scope, &snap.main_make())?
        } else {
            empty_rows
        };
        set(&mut scope, obj, "main_make_items", main_make)?;
    } else if !had {
        set(&mut scope, obj, "main_make_available", falsy)?;
        set(&mut scope, obj, "main_make_items", empty_rows)?;
    }
    if snap.has_reach() {
        let reach = reach_object(&mut scope, snap.reach())?;
        set(&mut scope, obj, "reach", reach)?;
    } else if !had {
        let reach = unavailable_reach(&mut scope)?;
        set(&mut scope, obj, "reach", reach)?;
    }
    if snap.has_collision() {
        let collision = collision_object(&mut scope, snap.collision())?;
        set(&mut scope, obj, "collision", collision)?;
    } else if !had {
        let collision = unavailable_collision(&mut scope)?;
        set(&mut scope, obj, "collision", collision)?;
    }
    if snap.has_attacked_by_player() {
        let attacked = v8::Boolean::new(&mut scope, snap.attacked_by_player());
        set(&mut scope, obj, "attacked_by_player", attacked.into())?;
    } else if !had {
        set(&mut scope, obj, "attacked_by_player", falsy)?;
    }
    if snap.has_widgets() {
        let widgets = widget_text_array(&mut scope, &snap.widgets())?;
        set(&mut scope, obj, "widgets", widgets)?;
    } else if !had {
        set(&mut scope, obj, "widgets", empty_rows)?;
    }
    if snap.has_quest_statuses_update() {
        if snap.quest_statuses_available() {
            let quests = quest_status_array(&mut scope, &snap.quest_statuses())?;
            set(&mut scope, obj, "quest_statuses", quests)?;
        } else {
            set(&mut scope, obj, "quest_statuses", none)?;
        }
    } else if snap.has_quest_statuses() {
        // Accept buffers from the additive vector-only draft as available.
        let quests = quest_status_array(&mut scope, &snap.quest_statuses())?;
        set(&mut scope, obj, "quest_statuses", quests)?;
    } else if !had {
        set(&mut scope, obj, "quest_statuses", none)?;
    }
    if snap.has_npc_boxes_update() {
        if snap.npc_boxes_available() {
            let boxes = npc_box_array(&mut scope, &snap.npc_boxes())?;
            set(&mut scope, obj, "npc_boxes", boxes)?;
        } else {
            set(&mut scope, obj, "npc_boxes", none)?;
        }
    } else if snap.has_npc_boxes() {
        // Accept buffers from an additive vector-only draft as available.
        let boxes = npc_box_array(&mut scope, &snap.npc_boxes())?;
        set(&mut scope, obj, "npc_boxes", boxes)?;
    } else if !had {
        set(&mut scope, obj, "npc_boxes", none)?;
    }
    if snap.has_self_chat() {
        let self_chat = match snap.self_chat() {
            Some("") | None => v8::null(&mut scope).into(),
            Some(text) => js_string(&mut scope, text)?,
        };
        set(&mut scope, obj, "self_chat", self_chat)?;
    } else if !had {
        set(&mut scope, obj, "self_chat", none)?;
    }
    if snap.has_hint_tile() {
        let hint = match snap.hint_tile() {
            Some((x, z)) => {
                let tile = v8::Object::new(&mut scope);
                let x = num(&mut scope, x as f64);
                set(&mut scope, tile, "x", x)?;
                let z = num(&mut scope, z as f64);
                set(&mut scope, tile, "z", z)?;
                tile.into()
            }
            None => v8::null(&mut scope).into(),
        };
        set(&mut scope, obj, "hint_tile", hint)?;
    } else if !had {
        set(&mut scope, obj, "hint_tile", none)?;
    }
    if snap.has_retaliate_controls() {
        let controls = match snap.retaliate_controls() {
            Some((on, off)) => {
                let controls = v8::Object::new(&mut scope);
                let on = num(&mut scope, on as f64);
                set(&mut scope, controls, "onComId", on)?;
                let off = num(&mut scope, off as f64);
                set(&mut scope, controls, "offComId", off)?;
                controls.into()
            }
            None => v8::null(&mut scope).into(),
        };
        set(&mut scope, obj, "retaliate_controls", controls)?;
    } else if !had {
        set(&mut scope, obj, "retaliate_controls", none)?;
    }
    if snap.has_hold() {
        let hold = v8::Boolean::new(&mut scope, snap.hold());
        set(&mut scope, obj, "hold", hold.into())?;
    } else if !had {
        set(&mut scope, obj, "hold", falsy)?;
    }
    // Mirror the host-owned gate onto `__rs2b0t_host.hold` every post
    // (hold is re-posted every tick — SEC-004). READ_ONLY so JS cannot
    // overwrite the posted value; tick_loop also gates on `host_hold`.
    let hold_host = v8::Boolean::new(&mut scope, host_hold);
    set_readonly(&mut scope, host, "hold", hold_host.into())?;
    if snap.has_canvas_width() && snap.canvas_width() > 0 && snap.canvas_height() > 0 {
        let w = snap.canvas_width();
        let h = snap.canvas_height();
        let rect = v8::Object::new(&mut scope);
        let zero = num(&mut scope, 0.0);
        set(&mut scope, rect, "left", zero)?;
        set(&mut scope, rect, "top", zero)?;
        set(&mut scope, rect, "x", zero)?;
        set(&mut scope, rect, "y", zero)?;
        let width = num(&mut scope, w as f64);
        let height = num(&mut scope, h as f64);
        set(&mut scope, rect, "width", width)?;
        set(&mut scope, rect, "height", height)?;
        set(&mut scope, rect, "right", width)?;
        set(&mut scope, rect, "bottom", height)?;
        set_readonly(&mut scope, host, "canvasRect", rect.into())?;
    } else {
        delete_key(&mut scope, host, "canvasRect")?;
    }
    if snap.has_ours() {
        let ours = v8::Boolean::new(&mut scope, snap.ours());
        set(&mut scope, obj, "ours", ours.into())?;
        set_readonly(&mut scope, host, "ours", ours.into())?;
    } else if !had {
        set(&mut scope, obj, "ours", falsy)?;
        set_readonly(&mut scope, host, "ours", falsy)?;
    }
    if snap.has_npcs() {
        let npcs = scene_entity_array(&mut scope, &snap.npcs())?;
        set(&mut scope, obj, "npcs", npcs)?;
    } else if !had {
        set(&mut scope, obj, "npcs", empty_rows)?;
    }
    if snap.has_locs() {
        let locs = scene_entity_array(&mut scope, &snap.locs())?;
        set(&mut scope, obj, "locs", locs)?;
    } else if !had {
        set(&mut scope, obj, "locs", empty_rows)?;
    }
    if snap.has_players() {
        let players = scene_entity_array(&mut scope, &snap.players())?;
        set(&mut scope, obj, "players", players)?;
    } else if !had {
        set(&mut scope, obj, "players", empty_rows)?;
    }
    if snap.has_ground() {
        let ground = scene_entity_array(&mut scope, &snap.ground())?;
        set(&mut scope, obj, "ground", ground)?;
    } else if !had {
        set(&mut scope, obj, "ground", empty_rows)?;
    }
    if snap.has_equipment() {
        let equipment = row_array(&mut scope, &snap.equipment())?;
        set(&mut scope, obj, "equipment", equipment)?;
    } else if !had {
        set(&mut scope, obj, "equipment", empty_rows)?;
    }
    if snap.has_chat_open() {
        let chat_open = v8::Boolean::new(&mut scope, snap.chat_open());
        set(&mut scope, obj, "chat_open", chat_open.into())?;
    } else if !had {
        set(&mut scope, obj, "chat_open", falsy)?;
    }
    if snap.has_chat_continue() {
        let chat_continue = v8::Boolean::new(&mut scope, snap.chat_continue());
        set(&mut scope, obj, "chat_continue", chat_continue.into())?;
    } else if !had {
        set(&mut scope, obj, "chat_continue", falsy)?;
    }
    if snap.has_chat_text() {
        let chat_text = match snap.chat_text() {
            Some("") | None => v8::null(&mut scope).into(),
            Some(s) => js_string(&mut scope, s)?,
        };
        set(&mut scope, obj, "chat_text", chat_text)?;
    } else if !had {
        set(&mut scope, obj, "chat_text", none)?;
    }
    if snap.has_chat_options() {
        let chat_options = chat_option_array(&mut scope, &snap.chat_options())?;
        set(&mut scope, obj, "chat_options", chat_options)?;
    } else if !had {
        set(&mut scope, obj, "chat_options", empty_rows)?;
    }
    if snap.has_side_tab() {
        let side_tab = num(&mut scope, snap.side_tab() as f64);
        set(&mut scope, obj, "side_tab", side_tab)?;
    } else if !had {
        let neg = num(&mut scope, -1.0);
        set(&mut scope, obj, "side_tab", neg)?;
    }
    if snap.has_varps() {
        let varps = varp_array(&mut scope, &snap.varps())?;
        set(&mut scope, obj, "varps", varps)?;
    } else if !had {
        set(&mut scope, obj, "varps", empty_rows)?;
    }
    if snap.has_combat_styles() {
        let combat_styles = combat_style_array(&mut scope, &snap.combat_styles())?;
        set(&mut scope, obj, "combat_styles", combat_styles)?;
    } else if !had {
        set(&mut scope, obj, "combat_styles", empty_rows)?;
    }
    if snap.has_run_energy() {
        let run_energy = num(&mut scope, snap.run_energy() as f64);
        set(&mut scope, obj, "run_energy", run_energy)?;
    } else if !had {
        let zero = num(&mut scope, 0.0);
        set(&mut scope, obj, "run_energy", zero)?;
    }
    if snap.has_run_enabled() {
        let run_enabled = v8::Boolean::new(&mut scope, snap.run_enabled());
        set(&mut scope, obj, "run_enabled", run_enabled.into())?;
    } else if !had {
        set(&mut scope, obj, "run_enabled", falsy)?;
    }
    if snap.has_retaliate_enabled() {
        let retaliate_enabled = v8::Boolean::new(&mut scope, snap.retaliate_enabled());
        set(
            &mut scope,
            obj,
            "retaliate_enabled",
            retaliate_enabled.into(),
        )?;
    } else if !had {
        set(&mut scope, obj, "retaliate_enabled", falsy)?;
    }
    if snap.has_my_name() {
        let my_name = match snap.my_name() {
            Some("") | None => v8::null(&mut scope).into(),
            Some(s) => js_string(&mut scope, s)?,
        };
        set(&mut scope, obj, "my_name", my_name)?;
    } else if !had {
        set(&mut scope, obj, "my_name", none)?;
    }
    if snap.has_in_combat() {
        let in_combat = v8::Boolean::new(&mut scope, snap.in_combat());
        set(&mut scope, obj, "in_combat", in_combat.into())?;
    } else if !had {
        set(&mut scope, obj, "in_combat", falsy)?;
    }
    if snap.has_animating() {
        let animating = v8::Boolean::new(&mut scope, snap.animating());
        set(&mut scope, obj, "animating", animating.into())?;
    } else if !had {
        set(&mut scope, obj, "animating", falsy)?;
    }
    if snap.has_main_modal_id() {
        let main_modal_id = num(&mut scope, snap.main_modal_id() as f64);
        set(&mut scope, obj, "main_modal_id", main_modal_id)?;
    } else if !had {
        let neg = num(&mut scope, -1.0);
        set(&mut scope, obj, "main_modal_id", neg)?;
    }
    if snap.has_chat_modal_id() {
        let chat_modal_id = num(&mut scope, snap.chat_modal_id() as f64);
        set(&mut scope, obj, "chat_modal_id", chat_modal_id)?;
    } else if !had {
        let neg = num(&mut scope, -1.0);
        set(&mut scope, obj, "chat_modal_id", neg)?;
    }
    if snap.has_make_products() {
        let make_products = make_product_array(&mut scope, &snap.make_products())?;
        set(&mut scope, obj, "make_products", make_products)?;
    } else if !had {
        set(&mut scope, obj, "make_products", empty_rows)?;
    }
    if snap.has_side_tab_ifaces() {
        let ifaces = side_tab_iface_array(&mut scope, &snap.side_tab_ifaces())?;
        set(&mut scope, obj, "side_tab_ifaces", ifaces)?;
    } else if !had {
        set(&mut scope, obj, "side_tab_ifaces", empty_rows)?;
    }
    if snap.has_spell_buttons() {
        let spell_buttons = combat_style_array(&mut scope, &snap.spell_buttons())?;
        set(&mut scope, obj, "spell_buttons", spell_buttons)?;
    } else if !had {
        set(&mut scope, obj, "spell_buttons", empty_rows)?;
    }
    if snap.has_chat_lines() {
        let chat_lines = chat_line_array(&mut scope, &snap.chat_lines())?;
        set(&mut scope, obj, "chat_lines", chat_lines)?;
    } else if !had {
        set(&mut scope, obj, "chat_lines", empty_rows)?;
    }
    let snapshot = obj.into();
    set(&mut scope, host, "snapshot", snapshot)
}

pub(super) fn materialize_settings_bag(runtime: &mut Runtime, json: &str) -> Result<(), String> {
    runtime
        .eval::<()>(format!("globalThis.__rs2b0t_host.settingsBag = {json};"))
        .map_err(|e| format!("settings bag: {e}"))
}

fn native_event_object<'s>(
    scope: &mut v8::HandleScope<'s>,
    ev: &crate::events::NativeEvent,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let wrapped = v8::Object::new(scope);
    let type_name = js_string(scope, ev.type_name())?;
    set(scope, wrapped, "type", type_name)?;
    let payload = v8::Object::new(scope);
    match ev {
        crate::events::NativeEvent::ChatMessage {
            type_,
            username,
            text,
        } => {
            let type_value = num(scope, *type_ as f64);
            set(scope, payload, "type", type_value)?;
            match username {
                Some(name) => {
                    let value = js_string(scope, name)?;
                    set(scope, payload, "username", value)?;
                }
                None => {
                    let value = v8::undefined(scope).into();
                    set(scope, payload, "username", value)?;
                }
            }
            let text_value = js_string(scope, text)?;
            set(scope, payload, "text", text_value)?;
        }
        crate::events::NativeEvent::SkillXp {
            skill,
            name,
            xp,
            delta,
        } => {
            let skill_v = num(scope, *skill as f64);
            set(scope, payload, "skill", skill_v)?;
            let name_v = js_string(scope, name)?;
            set(scope, payload, "name", name_v)?;
            let xp_v = num(scope, *xp as f64);
            set(scope, payload, "xp", xp_v)?;
            let delta_v = num(scope, *delta as f64);
            set(scope, payload, "delta", delta_v)?;
        }
        crate::events::NativeEvent::InventoryChanged {
            slot,
            id,
            name,
            count,
            previous_id,
            previous_count,
        } => {
            let slot_v = num(scope, *slot as f64);
            set(scope, payload, "slot", slot_v)?;
            let id_v = num(scope, *id as f64);
            set(scope, payload, "id", id_v)?;
            match name {
                Some(n) => {
                    let name_v = js_string(scope, n)?;
                    set(scope, payload, "name", name_v)?;
                }
                None => {
                    let none = v8::null(scope).into();
                    set(scope, payload, "name", none)?;
                }
            }
            let count_v = num(scope, *count as f64);
            set(scope, payload, "count", count_v)?;
            let prev_id = num(scope, *previous_id as f64);
            set(scope, payload, "previousId", prev_id)?;
            let prev_count = num(scope, *previous_count as f64);
            set(scope, payload, "previousCount", prev_count)?;
        }
    }
    set(scope, wrapped, "payload", payload.into())?;
    Ok(wrapped.into())
}

pub(super) fn dispatch_native_events(
    runtime: &mut Runtime,
    events: &[crate::events::NativeEvent],
) -> Result<(), String> {
    if events.is_empty() {
        return Ok(());
    }
    {
        let context = runtime.deno_runtime().main_context();
        let mut scope = runtime.deno_runtime().handle_scope();
        let global = context.open(&mut scope).global(&mut scope);
        let arr = v8::Array::new(&mut scope, events.len() as i32);
        for (i, ev) in events.iter().enumerate() {
            let obj = native_event_object(&mut scope, ev)?;
            arr.set_index(&mut scope, i as u32, obj)
                .ok_or_else(|| "v8 array set failed".to_string())?;
        }
        let key = js_string(&mut scope, "__rs2b0t_native_event_batch")?;
        global
            .set(&mut scope, key, arr.into())
            .ok_or_else(|| "v8 set batch failed".to_string())?;
    }
    runtime
        .eval::<()>(
            "(() => { const b = globalThis.__rs2b0t_native_event_batch; globalThis.__rs2b0t_native_event_batch = null; const q = globalThis.__rs2b0t_pending_native_event_batch || (globalThis.__rs2b0t_pending_native_event_batch = []); q.push(...b); })()",
        )
        .map_err(|e| format!("{e}"))
}

fn js_string<'s>(
    scope: &mut v8::HandleScope<'s>,
    s: &str,
) -> Result<v8::Local<'s, v8::Value>, String> {
    v8::String::new(scope, s)
        .map(|v| v.into())
        .ok_or_else(|| "v8 string alloc failed".to_string())
}

fn num<'s>(scope: &mut v8::HandleScope<'s>, n: f64) -> v8::Local<'s, v8::Value> {
    v8::Number::new(scope, n).into()
}

fn set<'s>(
    scope: &mut v8::HandleScope<'s>,
    obj: v8::Local<'s, v8::Object>,
    key: &str,
    value: v8::Local<'s, v8::Value>,
) -> Result<(), String> {
    let key = js_string(scope, key)?;
    obj.set(scope, key, value)
        .ok_or_else(|| format!("v8 object set failed for {key:?}"))?;
    Ok(())
}

fn set_readonly<'s>(
    scope: &mut v8::HandleScope<'s>,
    obj: v8::Local<'s, v8::Object>,
    key: &str,
    value: v8::Local<'s, v8::Value>,
) -> Result<(), String> {
    let name =
        v8::String::new(scope, key).ok_or_else(|| "v8 string alloc failed".to_string())?;
    obj.define_own_property(scope, name.into(), value, v8::PropertyAttribute::READ_ONLY)
        .ok_or_else(|| format!("v8 define_own_property failed for {key}"))?;
    Ok(())
}

fn delete_key<'s>(
    scope: &mut v8::HandleScope<'s>,
    obj: v8::Local<'s, v8::Object>,
    key: &str,
) -> Result<(), String> {
    let name =
        v8::String::new(scope, key).ok_or_else(|| "v8 string alloc failed".to_string())?;
    obj.delete(scope, name.into())
        .ok_or_else(|| format!("v8 object delete failed for {key}"))?;
    Ok(())
}

/// One `{id, name, ops, count, noted, cert, component_id, slot}` row from ItemView.
fn row_object<'s>(
    scope: &mut v8::HandleScope<'s>,
    row: &crate::isolate_fb::RowReader<'_>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let o = v8::Object::new(scope);
    match row.name() {
        Some(name) => {
            let name = js_string(scope, name)?;
            set(scope, o, "name", name)?;
        }
        None => {
            let none = v8::null(scope);
            set(scope, o, "name", none.into())?;
        }
    }
    let count = num(scope, row.count() as f64);
    set(scope, o, "count", count)?;
    let id = num(scope, row.id() as f64);
    set(scope, o, "id", id)?;
    let ops = v8::Array::new(scope, row.ops().len() as i32);
    for (i, op) in row.ops().iter().enumerate() {
        let a = js_string(scope, op)?;
        ops.set_index(scope, i as u32, a)
            .ok_or_else(|| "v8 array set failed".to_string())?;
    }
    set(scope, o, "ops", ops.into())?;
    let noted = v8::Boolean::new(scope, row.noted());
    set(scope, o, "noted", noted.into())?;
    let cert = num(scope, row.cert() as f64);
    set(scope, o, "cert", cert)?;
    if row.has_component_id() {
        let component_id = num(scope, row.component_id() as f64);
        set(scope, o, "component_id", component_id)?;
    }
    if row.has_slot() {
        let slot = num(scope, row.slot() as f64);
        set(scope, o, "slot", slot)?;
    }
    Ok(o.into())
}

fn row_array<'s>(
    scope: &mut v8::HandleScope<'s>,
    rows: &[crate::isolate_fb::RowReader<'_>],
) -> Result<v8::Local<'s, v8::Value>, String> {
    let arr = v8::Array::new(scope, rows.len() as i32);
    for (i, row) in rows.iter().enumerate() {
        let row = row_object(scope, row)?;
        arr.set_index(scope, i as u32, row)
            .ok_or_else(|| "v8 array set failed".to_string())?;
    }
    Ok(arr.into())
}

fn stat_array<'s>(
    scope: &mut v8::HandleScope<'s>,
    stats: &[crate::isolate_fb::StatReader<'_>],
) -> Result<v8::Local<'s, v8::Value>, String> {
    let arr = v8::Array::new(scope, stats.len() as i32);
    for (i, st) in stats.iter().enumerate() {
        let o = v8::Object::new(scope);
        let index = num(scope, st.index() as f64);
        set(scope, o, "index", index)?;
        let name = js_string(scope, st.name())?;
        set(scope, o, "name", name)?;
        let xp = num(scope, st.xp() as f64);
        set(scope, o, "xp", xp)?;
        let base = num(scope, st.base() as f64);
        set(scope, o, "base", base)?;
        let effective = num(scope, st.effective() as f64);
        set(scope, o, "effective", effective)?;
        let obj = o.into();
        arr.set_index(scope, i as u32, obj)
            .ok_or_else(|| "v8 array set failed".to_string())?;
    }
    Ok(arr.into())
}

fn tile_object<'s>(
    scope: &mut v8::HandleScope<'s>,
    t: &crate::isolate_fb::TileReader<'_>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    tile_values_object(scope, t.x(), t.z(), t.level())
}

fn tile_values_object<'s>(
    scope: &mut v8::HandleScope<'s>,
    x_value: i32,
    z_value: i32,
    level_value: i32,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let o = v8::Object::new(scope);
    let x = num(scope, x_value as f64);
    set(scope, o, "x", x)?;
    let z = num(scope, z_value as f64);
    set(scope, o, "z", z)?;
    let level = num(scope, level_value as f64);
    set(scope, o, "level", level)?;
    Ok(o.into())
}

fn u32_array<'s>(
    scope: &mut v8::HandleScope<'s>,
    words: &[u32],
) -> Result<v8::Local<'s, v8::Value>, String> {
    let arr = v8::Array::new(scope, words.len() as i32);
    for (i, word) in words.iter().enumerate() {
        let n = num(scope, f64::from(*word));
        arr.set_index(scope, i as u32, n)
            .ok_or_else(|| "v8 array set failed".to_string())?;
    }
    Ok(arr.into())
}

fn u8_array<'s>(
    scope: &mut v8::HandleScope<'s>,
    bytes: &[u8],
) -> Result<v8::Local<'s, v8::Value>, String> {
    let arr = v8::Array::new(scope, bytes.len() as i32);
    for (i, byte) in bytes.iter().enumerate() {
        let n = num(scope, f64::from(*byte));
        arr.set_index(scope, i as u32, n)
            .ok_or_else(|| "v8 array set failed".to_string())?;
    }
    Ok(arr.into())
}

fn u16_array<'s>(
    scope: &mut v8::HandleScope<'s>,
    values: &[u16],
) -> Result<v8::Local<'s, v8::Value>, String> {
    let arr = v8::Array::new(scope, values.len() as i32);
    for (i, value) in values.iter().enumerate() {
        let n = num(scope, f64::from(*value));
        arr.set_index(scope, i as u32, n)
            .ok_or_else(|| "v8 array set failed".to_string())?;
    }
    Ok(arr.into())
}

fn unavailable_collision<'s>(
    scope: &mut v8::HandleScope<'s>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    collision_from_parts(scope, false, 0, 0, 0, 0, 0, &[])
}

fn collision_object<'s>(
    scope: &mut v8::HandleScope<'s>,
    collision: Option<crate::isolate_fb::CollisionReader<'_>>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let Some(c) = collision else {
        return unavailable_collision(scope);
    };
    let flags = c.flags();
    collision_from_parts(
        scope,
        c.available(),
        c.base_x(),
        c.base_z(),
        c.level(),
        c.width(),
        c.height(),
        &flags,
    )
}

fn collision_from_parts<'s>(
    scope: &mut v8::HandleScope<'s>,
    available: bool,
    base_x: i32,
    base_z: i32,
    level: i32,
    width: i32,
    height: i32,
    flags: &[i32],
) -> Result<v8::Local<'s, v8::Value>, String> {
    let o = v8::Object::new(scope);
    let available_v = v8::Boolean::new(scope, available).into();
    set(scope, o, "available", available_v)?;
    let base_x_v = num(scope, base_x as f64);
    set(scope, o, "base_x", base_x_v)?;
    let base_z_v = num(scope, base_z as f64);
    set(scope, o, "base_z", base_z_v)?;
    let level_v = num(scope, level as f64);
    set(scope, o, "level", level_v)?;
    let width_v = num(scope, width as f64);
    set(scope, o, "width", width_v)?;
    let height_v = num(scope, height as f64);
    set(scope, o, "height", height_v)?;
    let view_flags = if available { flags } else { &[] };
    let flags_v = collision_flags_view(scope, view_flags)?;
    set(scope, o, "flags", flags_v)?;
    Ok(o.into())
}

fn collision_flags_view<'s>(
    scope: &mut v8::HandleScope<'s>,
    flags: &[i32],
) -> Result<v8::Local<'s, v8::Value>, String> {
    let nbytes = flags.len().saturating_mul(4);
    let ab = v8::ArrayBuffer::new(scope, nbytes);
    if nbytes > 0 {
        let backing = ab.get_backing_store();
        if let Some(ptr) = backing.data() {
            let dest = ptr.as_ptr() as *mut u8;
            for (i, flag) in flags.iter().enumerate() {
                let bytes = flag.to_le_bytes();
                unsafe {
                    std::ptr::copy_nonoverlapping(bytes.as_ptr(), dest.add(i * 4), 4);
                }
            }
        }
    }
    let u8a = v8::Uint8Array::new(scope, ab, 0, nbytes)
        .ok_or_else(|| "uint8 collision flags".to_string())?;
    let global = scope.get_current_context().global(scope);
    let key = v8::String::new(scope, "__rs2b0t_flags_view")
        .ok_or_else(|| "flags view name".to_string())?;
    let factory = global
        .get(scope, key.into())
        .ok_or_else(|| "missing flags view".to_string())?;
    let factory = v8::Local::<v8::Function>::try_from(factory)
        .map_err(|_| "flags view is not a function".to_string())?;
    let recv = v8::undefined(scope).into();
    factory
        .call(scope, recv, &[u8a.into()])
        .ok_or_else(|| "flags view call".to_string())
}

fn unavailable_reach<'s>(
    scope: &mut v8::HandleScope<'s>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let o = v8::Object::new(scope);
    let falsy: v8::Local<v8::Value> = v8::Boolean::new(scope, false).into();
    set(scope, o, "available", falsy)?;
    let zero = num(scope, 0.0);
    set(scope, o, "base_x", zero)?;
    set(scope, o, "base_z", zero)?;
    set(scope, o, "level", zero)?;
    set(scope, o, "width", zero)?;
    set(scope, o, "height", zero)?;
    let empty = v8::Array::new(scope, 0);
    set(scope, o, "walkable", empty.into())?;
    let empty = v8::Array::new(scope, 0);
    set(scope, o, "reachable", empty.into())?;
    let empty = v8::Array::new(scope, 0);
    set(scope, o, "reachable_adj", empty.into())?;
    let empty = v8::Array::new(scope, 0);
    set(scope, o, "exact_rank", empty.into())?;
    let empty = v8::Array::new(scope, 0);
    set(scope, o, "adjacent_rank", empty.into())?;
    let empty = v8::Array::new(scope, 0);
    set(scope, o, "step", empty.into())?;
    Ok(o.into())
}

fn reach_object<'s>(
    scope: &mut v8::HandleScope<'s>,
    reach: Option<crate::isolate_fb::ReachReader<'_>>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let Some(r) = reach else {
        return unavailable_reach(scope);
    };
    let o = v8::Object::new(scope);
    let available = v8::Boolean::new(scope, r.available());
    set(scope, o, "available", available.into())?;
    let base_x = num(scope, r.base_x() as f64);
    set(scope, o, "base_x", base_x)?;
    let base_z = num(scope, r.base_z() as f64);
    set(scope, o, "base_z", base_z)?;
    let level = num(scope, r.level() as f64);
    set(scope, o, "level", level)?;
    let width = num(scope, r.width() as f64);
    set(scope, o, "width", width)?;
    let height = num(scope, r.height() as f64);
    set(scope, o, "height", height)?;
    let walkable = u32_array(scope, &r.walkable())?;
    set(scope, o, "walkable", walkable)?;
    let reachable = u32_array(scope, &r.reachable())?;
    set(scope, o, "reachable", reachable)?;
    let reachable_adj = u32_array(scope, &r.reachable_adj())?;
    set(scope, o, "reachable_adj", reachable_adj)?;
    let exact_rank = u16_array(scope, &r.exact_rank())?;
    set(scope, o, "exact_rank", exact_rank)?;
    let adjacent_rank = u16_array(scope, &r.adjacent_rank())?;
    set(scope, o, "adjacent_rank", adjacent_rank)?;
    let step = u8_array(scope, &r.step())?;
    set(scope, o, "step", step)?;
    Ok(o.into())
}

fn nearest_booth_object<'s>(
    scope: &mut v8::HandleScope<'s>,
    nb: &crate::isolate_fb::NearestBoothReader<'_>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let o = v8::Object::new(scope);
    let x = num(scope, nb.x() as f64);
    set(scope, o, "x", x)?;
    let z = num(scope, nb.z() as f64);
    set(scope, o, "z", z)?;
    let level = num(scope, nb.level() as f64);
    set(scope, o, "level", level)?;
    let id = num(scope, nb.id() as f64);
    set(scope, o, "id", id)?;
    let name = js_string(scope, nb.name())?;
    set(scope, o, "name", name)?;
    let op = js_string(scope, nb.op())?;
    set(scope, o, "op", op)?;
    Ok(o.into())
}

fn tile_array<'s>(
    scope: &mut v8::HandleScope<'s>,
    tiles: &[crate::isolate_fb::TileReader<'_>],
) -> Result<v8::Local<'s, v8::Value>, String> {
    let arr = v8::Array::new(scope, tiles.len() as i32);
    for (i, t) in tiles.iter().enumerate() {
        let t = tile_object(scope, t)?;
        arr.set_index(scope, i as u32, t)
            .ok_or_else(|| "v8 array set failed".to_string())?;
    }
    Ok(arr.into())
}

fn scene_entity_object<'s>(
    scope: &mut v8::HandleScope<'s>,
    ent: &crate::isolate_fb::SceneEntityReader<'_>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let o = v8::Object::new(scope);
    let index = num(scope, ent.index() as f64);
    set(scope, o, "index", index)?;
    let id = num(scope, ent.id() as f64);
    set(scope, o, "id", id)?;
    match ent.name() {
        Some(name) => {
            let name = js_string(scope, name)?;
            set(scope, o, "name", name)?;
        }
        None => {
            let none = v8::null(scope);
            set(scope, o, "name", none.into())?;
        }
    }
    let x = num(scope, ent.x() as f64);
    set(scope, o, "x", x)?;
    let z = num(scope, ent.z() as f64);
    set(scope, o, "z", z)?;
    let level = num(scope, ent.level() as f64);
    set(scope, o, "level", level)?;
    // ClientAdapter reader.locs() keeps the native flat fields for
    // existing consumers, while compat Loc consumers read the same
    // coordinates through the nested `tile` shape.
    let tile = tile_values_object(scope, ent.x(), ent.z(), ent.level())?;
    set(scope, o, "tile", tile)?;
    let distance = num(scope, ent.distance() as f64);
    set(scope, o, "distance", distance)?;
    let health = num(scope, ent.health() as f64);
    set(scope, o, "health", health)?;
    let max_health = num(scope, ent.max_health() as f64);
    set(scope, o, "max_health", max_health)?;
    set(scope, o, "totalHealth", max_health)?;
    let in_combat = v8::Boolean::new(scope, ent.in_combat());
    set(scope, o, "in_combat", in_combat.into())?;
    let animating = v8::Boolean::new(scope, ent.animating());
    set(scope, o, "animating", animating.into())?;
    let actions = v8::Array::new(scope, ent.actions().len() as i32);
    for (i, action) in ent.actions().iter().enumerate() {
        let a = js_string(scope, action)?;
        actions
            .set_index(scope, i as u32, a)
            .ok_or_else(|| "v8 array set failed".to_string())?;
    }
    set(scope, o, "actions", actions.into())?;
    let reachable = v8::Boolean::new(scope, ent.reachable());
    set(scope, o, "reachable", reachable.into())?;
    let reachable_adj = v8::Boolean::new(scope, ent.reachable_adj());
    set(scope, o, "reachable_adj", reachable_adj.into())?;
    let combat_level = num(scope, ent.combat_level() as f64);
    set(scope, o, "combat_level", combat_level)?;
    let target_kind = num(scope, ent.target_kind() as f64);
    set(scope, o, "target_kind", target_kind)?;
    let target_index = num(scope, ent.target_index() as f64);
    set(scope, o, "target_index", target_index)?;
    Ok(o.into())
}

fn scene_entity_array<'s>(
    scope: &mut v8::HandleScope<'s>,
    ents: &[crate::isolate_fb::SceneEntityReader<'_>],
) -> Result<v8::Local<'s, v8::Value>, String> {
    let arr = v8::Array::new(scope, ents.len() as i32);
    for (i, ent) in ents.iter().enumerate() {
        let ent = scene_entity_object(scope, ent)?;
        arr.set_index(scope, i as u32, ent)
            .ok_or_else(|| "v8 array set failed".to_string())?;
    }
    Ok(arr.into())
}

fn chat_option_array<'s>(
    scope: &mut v8::HandleScope<'s>,
    opts: &[crate::isolate_fb::ChatOptionReader<'_>],
) -> Result<v8::Local<'s, v8::Value>, String> {
    let arr = v8::Array::new(scope, opts.len() as i32);
    for (i, opt) in opts.iter().enumerate() {
        let o = v8::Object::new(scope);
        let text = js_string(scope, opt.text())?;
        set(scope, o, "text", text)?;
        let obj = o.into();
        arr.set_index(scope, i as u32, obj)
            .ok_or_else(|| "v8 array set failed".to_string())?;
    }
    Ok(arr.into())
}

fn make_product_array<'s>(
    scope: &mut v8::HandleScope<'s>,
    products: &[crate::isolate_fb::MakeProductReader<'_>],
) -> Result<v8::Local<'s, v8::Value>, String> {
    let arr = v8::Array::new(scope, products.len() as i32);
    for (i, product) in products.iter().enumerate() {
        let o = v8::Object::new(scope);
        let name = js_string(scope, product.name())?;
        set(scope, o, "name", name)?;
        let oid = num(scope, product.object_id() as f64);
        set(scope, o, "object_id", oid)?;
        let buttons = v8::Array::new(scope, product.buttons().len() as i32);
        for (j, btn) in product.buttons().iter().enumerate() {
            let b = v8::Object::new(scope);
            let qty = num(scope, btn.qty() as f64);
            set(scope, b, "qty", qty)?;
            let com_id = num(scope, btn.com_id() as f64);
            set(scope, b, "comId", com_id)?;
            buttons
                .set_index(scope, j as u32, b.into())
                .ok_or_else(|| "v8 array set failed".to_string())?;
        }
        set(scope, o, "buttons", buttons.into())?;
        arr.set_index(scope, i as u32, o.into())
            .ok_or_else(|| "v8 array set failed".to_string())?;
    }
    Ok(arr.into())
}

fn varp_array<'s>(
    scope: &mut v8::HandleScope<'s>,
    varps: &[crate::isolate_fb::VarpReader<'_>],
) -> Result<v8::Local<'s, v8::Value>, String> {
    let arr = v8::Array::new(scope, varps.len() as i32);
    for (i, v) in varps.iter().enumerate() {
        let o = v8::Object::new(scope);
        let index = num(scope, v.index() as f64);
        set(scope, o, "index", index)?;
        let value = num(scope, v.value() as f64);
        set(scope, o, "value", value)?;
        let obj = o.into();
        arr.set_index(scope, i as u32, obj)
            .ok_or_else(|| "v8 array set failed".to_string())?;
    }
    Ok(arr.into())
}

fn combat_style_array<'s>(
    scope: &mut v8::HandleScope<'s>,
    styles: &[crate::isolate_fb::CombatStyleReader<'_>],
) -> Result<v8::Local<'s, v8::Value>, String> {
    let arr = v8::Array::new(scope, styles.len() as i32);
    for (i, st) in styles.iter().enumerate() {
        let o = v8::Object::new(scope);
        let mode = num(scope, st.mode() as f64);
        set(scope, o, "mode", mode)?;
        let label = js_string(scope, st.label())?;
        set(scope, o, "label", label)?;
        let component_id = num(scope, st.component_id() as f64);
        set(scope, o, "component_id", component_id)?;
        let obj = o.into();
        arr.set_index(scope, i as u32, obj)
            .ok_or_else(|| "v8 array set failed".to_string())?;
    }
    Ok(arr.into())
}

fn side_tab_iface_array<'s>(
    scope: &mut v8::HandleScope<'s>,
    tabs: &[crate::isolate_fb::SideTabIfaceReader<'_>],
) -> Result<v8::Local<'s, v8::Value>, String> {
    let arr = v8::Array::new(scope, tabs.len() as i32);
    for (i, t) in tabs.iter().enumerate() {
        let o = v8::Object::new(scope);
        let index = num(scope, t.index() as f64);
        set(scope, o, "index", index)?;
        let id = num(scope, t.id() as f64);
        set(scope, o, "id", id)?;
        arr.set_index(scope, i as u32, o.into())
            .ok_or_else(|| "v8 array set failed".to_string())?;
    }
    Ok(arr.into())
}

fn chat_line_array<'s>(
    scope: &mut v8::HandleScope<'s>,
    lines: &[crate::isolate_fb::ChatLineReader<'_>],
) -> Result<v8::Local<'s, v8::Value>, String> {
    let arr = v8::Array::new(scope, lines.len() as i32);
    for (i, line) in lines.iter().enumerate() {
        let o = v8::Object::new(scope);
        let seq = num(scope, line.seq() as f64);
        set(scope, o, "seq", seq)?;
        let text = js_string(scope, line.text())?;
        set(scope, o, "text", text)?;
        let type_ = num(scope, line.type_() as f64);
        set(scope, o, "type", type_)?;
        if let Some(username) = line.username() {
            let username = js_string(scope, username)?;
            set(scope, o, "username", username)?;
        }
        arr.set_index(scope, i as u32, o.into())
            .ok_or_else(|| "v8 array set failed".to_string())?;
    }
    Ok(arr.into())
}

fn widget_text_array<'s>(
    scope: &mut v8::HandleScope<'s>,
    rows: &[crate::isolate_fb::WidgetTextReader<'_>],
) -> Result<v8::Local<'s, v8::Value>, String> {
    let arr = v8::Array::new(scope, rows.len() as i32);
    for (i, row) in rows.iter().enumerate() {
        let o = v8::Object::new(scope);
        let component_id = num(scope, row.component_id() as f64);
        set(scope, o, "component_id", component_id)?;
        let text = js_string(scope, row.text())?;
        set(scope, o, "text", text)?;
        arr.set_index(scope, i as u32, o.into())
            .ok_or_else(|| "v8 array set failed".to_string())?;
    }
    Ok(arr.into())
}

fn quest_status_array<'s>(
    scope: &mut v8::HandleScope<'s>,
    rows: &[crate::isolate_fb::QuestStatusReader<'_>],
) -> Result<v8::Local<'s, v8::Value>, String> {
    let arr = v8::Array::new(scope, rows.len() as i32);
    for (i, row) in rows.iter().enumerate() {
        let o = v8::Object::new(scope);
        let name = js_string(scope, row.name())?;
        set(scope, o, "name", name)?;
        let status = js_string(scope, row.status())?;
        set(scope, o, "status", status)?;
        arr.set_index(scope, i as u32, o.into())
            .ok_or_else(|| "v8 array set failed".to_string())?;
    }
    Ok(arr.into())
}

fn npc_box_array<'s>(
    scope: &mut v8::HandleScope<'s>,
    rows: &[crate::isolate_fb::NpcBoxReader<'_>],
) -> Result<v8::Local<'s, v8::Value>, String> {
    let arr = v8::Array::new(scope, rows.len() as i32);
    for (i, row) in rows.iter().enumerate() {
        let o = v8::Object::new(scope);
        let index = num(scope, row.index() as f64);
        set(scope, o, "index", index)?;
        let points = row.points();
        let point_arr = v8::Array::new(scope, points.len() as i32);
        for (j, (x, y)) in points.into_iter().enumerate() {
            let point = v8::Object::new(scope);
            let x = num(scope, x as f64);
            set(scope, point, "x", x)?;
            let y = num(scope, y as f64);
            set(scope, point, "y", y)?;
            point_arr
                .set_index(scope, j as u32, point.into())
                .ok_or_else(|| "v8 array set failed".to_string())?;
        }
        set(scope, o, "points", point_arr.into())?;
        arr.set_index(scope, i as u32, o.into())
            .ok_or_else(|| "v8 array set failed".to_string())?;
    }
    Ok(arr.into())
}

fn bank_stand_array<'s>(
    scope: &mut v8::HandleScope<'s>,
    stands: &[crate::isolate_fb::BankStandReader<'_>],
) -> Result<v8::Local<'s, v8::Value>, String> {
    let arr = v8::Array::new(scope, stands.len() as i32);
    for (i, s) in stands.iter().enumerate() {
        let o = v8::Object::new(scope);
        let name = js_string(scope, s.name())?;
        set(scope, o, "name", name)?;
        let x = num(scope, s.x() as f64);
        set(scope, o, "x", x)?;
        let z = num(scope, s.z() as f64);
        set(scope, o, "z", z)?;
        let level = num(scope, s.level() as f64);
        set(scope, o, "level", level)?;
        let kind = js_string(scope, s.kind())?;
        set(scope, o, "kind", kind)?;
        let op = num(scope, s.op() as f64);
        set(scope, o, "op", op)?;
        match s.choose() {
            Some(choose) => {
                let choose = js_string(scope, choose)?;
                set(scope, o, "choose", choose)?;
            }
            None => {
                let none = v8::null(scope);
                set(scope, o, "choose", none.into())?;
            }
        }
        let obj = o.into();
        arr.set_index(scope, i as u32, obj)
            .ok_or_else(|| "v8 array set failed".to_string())?;
    }
    Ok(arr.into())
}

fn bank_approach_array<'s>(
    scope: &mut v8::HandleScope<'s>,
    rows: &[crate::isolate_fb::BankApproachReader<'_>],
) -> Result<v8::Local<'s, v8::Value>, String> {
    let arr = v8::Array::new(scope, rows.len() as i32);
    for (i, row) in rows.iter().enumerate() {
        let o = v8::Object::new(scope);
        let loc_id = num(scope, row.loc_id() as f64);
        set(scope, o, "loc_id", loc_id)?;
        let x = num(scope, row.x() as f64);
        set(scope, o, "x", x)?;
        let z = num(scope, row.z() as f64);
        set(scope, o, "z", z)?;
        let level = num(scope, row.level() as f64);
        set(scope, o, "level", level)?;
        let can_operate = v8::Boolean::new(scope, row.can_operate());
        set(scope, o, "can_operate", can_operate.into())?;
        let dest_ok = v8::Boolean::new(scope, row.dest_ok());
        set(scope, o, "dest_ok", dest_ok.into())?;
        let dest_x = num(scope, row.dest_x() as f64);
        set(scope, o, "dest_x", dest_x)?;
        let dest_z = num(scope, row.dest_z() as f64);
        set(scope, o, "dest_z", dest_z)?;
        let dest_level = num(scope, row.dest_level() as f64);
        set(scope, o, "dest_level", dest_level)?;
        arr.set_index(scope, i as u32, o.into())
            .ok_or_else(|| "v8 array set failed".to_string())?;
    }
    Ok(arr.into())
}

fn inspect_hop_array<'s>(
    scope: &mut v8::HandleScope<'s>,
    rows: &[crate::isolate_fb::InspectHopReader<'_>],
) -> Result<v8::Local<'s, v8::Value>, String> {
    let arr = v8::Array::new(scope, rows.len() as i32);
    for (i, row) in rows.iter().enumerate() {
        let o = v8::Object::new(scope);
        let kind = js_string(scope, row.kind())?;
        set(scope, o, "kind", kind)?;
        let loc_id = num(scope, row.loc_id() as f64);
        set(scope, o, "locId", loc_id)?;
        let loc_name = js_string(scope, row.loc_name())?;
        set(scope, o, "locName", loc_name)?;
        let action = js_string(scope, row.action())?;
        set(scope, o, "action", action)?;
        let option = num(scope, row.option() as f64);
        set(scope, o, "option", option)?;
        let from = v8::Object::new(scope);
        let from_x = num(scope, row.from_x() as f64);
        set(scope, from, "x", from_x)?;
        let from_z = num(scope, row.from_z() as f64);
        set(scope, from, "z", from_z)?;
        let from_level = num(scope, row.from_level() as f64);
        set(scope, from, "level", from_level)?;
        set(scope, o, "from", from.into())?;
        let to = v8::Object::new(scope);
        let to_x = num(scope, row.to_x() as f64);
        set(scope, to, "x", to_x)?;
        let to_z = num(scope, row.to_z() as f64);
        set(scope, to, "z", to_z)?;
        let to_level = num(scope, row.to_level() as f64);
        set(scope, to, "level", to_level)?;
        set(scope, o, "to", to.into())?;
        let ticks = num(scope, row.ticks() as f64);
        set(scope, o, "ticks", ticks)?;
        arr.set_index(scope, i as u32, o.into())
            .ok_or_else(|| "v8 array set failed".to_string())?;
    }
    Ok(arr.into())
}
