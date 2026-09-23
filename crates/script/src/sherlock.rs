//! Compiled solve-clue card (`Sherlock`): play, identify, dispatch.
//!
//! The clue machine is [`crate::clue`] — the same token / identify / phase /
//! clock machine the frozen JS wrapper calls through `__rs2b0t_clue`, running
//! on this slot's pump thread. This module owns no second solver: no
//! membership table, no trail table, no classify, no phase and no clock of
//! its own. It owns the two things that wrapper owned besides the machine:
//! marshalling the observed frame onto the machine's payload, and mapping the
//! machine's own verbs onto this slot's interact queue.
//!
//! One [`Script::tick`] is one wrapper `execute()` iteration:
//!
//! * No live session: the `begin` call. A refusal (no held membership, no
//!   selected pin) leaves the card idle and it asks again next tick; a
//!   `token` answer starts the session and this tick keeps iterating.
//! * Live: `next` calls until the machine waits, yields, ends the session or
//!   enqueues one verb — the one verb per tick the wrapper's own
//!   `delayTicks(1)` gives.
//! * `callback.enabled` is the machine's own gate question. The answer is
//!   `true` while this card is started (a compiled card ticks only while the
//!   slot runs it, and there is no settings `enabled` flag); `resume` then
//!   rides exactly the one following call.
//! * `callback.log` and `callback.setStatus` are the machine's status lines.
//!   This slice has no compiled log or status channel (paint/status is W10),
//!   so those kinds are answered and their message is not forwarded: nothing
//!   here rewrites them into a verb or a completion.
//! * `none-held` on a live session is the trail end. The session ends and the
//!   card's local solved count advances. The machine never emits `clue solved`
//!   and this module never invents one — no `done`, no completion line.
//!
//! Every page is read fail-closed from the observed frame
//! ([`ScriptCtx::snapshot`], [`ScriptCtx::here`], [`ScriptCtx::obj_names`] and
//! the ctx's [`crate::CompiledTick`]): an absent slot is omitted rather than
//! defaulted, so the machine never reads an invented `-1`, `28` or slot `0`.
//!
//! Out of scope for this card, on purpose: deposit, talk, keyboard, the trio
//! and gate supply, the death envelope, the duel `3554` family, honor-SETTINGS
//! and loadout provisioning. Those stay with the frozen card and its later
//! slices.

use api::game_data::SelectedGameData;
use api::snapshot::{ActorKind, ActorTargetView, GameSnapshot};
use serde_json::{json, Map, Value};

use crate::ctx::{Script, ScriptCtx};
use crate::shim::InteractReq;

/// The `generation` every call posts. The machine captures it at `begin` and
/// requires the same value back on every `next`; the compiled session is
/// aborted on its own pump instead ([`crate::slot::SlotScript`]), so the stamp
/// repeats the wrapper's own literal rather than carrying a session identity
/// of its own.
const GENERATION: u64 = 0;

/// The machine's own reason for a session whose held membership is gone: the
/// refusal a begin with nothing held gets, and the trail end a live session
/// finishes on.
const NONE_HELD: &str = "none-held";

/// How many machine calls one tick may make before it waits for the next one.
/// The machine answers a gate question, a log line and a status line at most
/// once each before it waits, yields or enqueues, so four handoffs is every
/// callback path; the bound is what keeps a changed machine from parking the
/// pump thread on one observed frame.
const CALLBACK_HANDOFFS: usize = 4;

/// The compiled solve-clue card.
#[derive(Default)]
pub struct Sherlock {
    /// The live session token; `None` while there is no session to continue.
    token: Option<u64>,
    /// Clue steps this card finished: the local count only, reported nowhere
    /// yet (the machine's own status lines are not a completion signal).
    solved: u32,
}

impl Script for Sherlock {
    fn name(&self) -> &str {
        "Sherlock"
    }

    fn tick(&mut self, ctx: &mut ScriptCtx<'_>) {
        // A verb needs somewhere to go: without this slot's queue the machine
        // is not asked at all, so a step it dispatched cannot be lost behind a
        // session that advanced anyway. The queue comes back whatever the
        // tick does.
        let Some(mut sink) = ctx.compiled.interacts.take() else {
            return;
        };
        self.pump(ctx, &mut sink);
        ctx.compiled.interacts = Some(sink);
    }
}

impl Sherlock {
    /// One pump iteration: the `begin` call while there is no live session,
    /// then one `execute()` iteration over it.
    fn pump(&mut self, ctx: &ScriptCtx<'_>, sink: &mut Vec<InteractReq>) {
        let selected = ctx.compiled.selected;
        let token = match self.token {
            Some(token) => token,
            None => {
                let begin = crate::clue::dispatch(selected, &begin_payload(ctx));
                let Some(started) = answered_token(&begin) else {
                    // Refused: nothing held this tick (or no selected pin).
                    // Idle, and the next tick asks the same question again.
                    return;
                };
                self.token = Some(started);
                started
            }
        };
        let page = next_payload(ctx, token);
        self.iterate(selected, page, sink);
    }

    /// One `execute()` iteration over the live token: the machine is called
    /// until it waits, yields, ends the session or enqueues one verb.
    ///
    /// The call-time page is marshalled once — the observed frame is fixed for
    /// this tick, and the machine reads the payload without retaining it — and
    /// only the `resume` answer is added and dropped around the one call it
    /// belongs to.
    fn iterate(
        &mut self,
        selected: Option<&SelectedGameData>,
        mut page: Value,
        sink: &mut Vec<InteractReq>,
    ) {
        let mut resume = false;
        for _ in 0..CALLBACK_HANDOFFS {
            let Some(fields) = page.as_object_mut() else {
                return;
            };
            if resume {
                fields.insert("resume".into(), Value::Bool(true));
            } else {
                fields.remove("resume");
            }
            let answer = crate::clue::dispatch(selected, &page);
            resume = false;
            let kind = answer.get("kind").and_then(Value::as_str).unwrap_or("");
            match kind {
                // The gate question: true while this card is started, and the
                // answer rides the very next call only.
                "callback.enabled" => resume = true,
                // The machine's own progress and status lines: answered, not
                // a verb, and this slice has no channel for their message.
                "callback.log" | "callback.setStatus" => {}
                // Frozen, or the cooperative interrupt. The token lives; the
                // session resumes on a later tick.
                "wait" | "yield" => return,
                "aborted" => {
                    let reason = answer.get("reason").and_then(Value::as_str).unwrap_or("");
                    self.end(reason);
                    return;
                }
                kind => {
                    // One enqueue per tick, exactly like the wrapper's own
                    // `delayTicks(1)`; an unknown kind is not a verb and is
                    // not enqueued as one either.
                    enqueue(sink, kind, &answer);
                    return;
                }
            }
        }
    }

    /// End the live session. `none-held` is the trail end, and the one end
    /// this card counts locally; every other reason — a token the machine no
    /// longer has, the abort a stop or a reset left, a pin that went missing —
    /// just ends it. Nothing is emitted either way: the machine owns
    /// completion, and this module never posts `clue solved`.
    fn end(&mut self, reason: &str) {
        if reason == NONE_HELD {
            self.solved = self.solved.saturating_add(1);
        }
        self.token = None;
    }
}

/// The `begin` call's payload: the required page, and nothing else.
fn begin_payload(ctx: &ScriptCtx<'_>) -> Value {
    json!({
        "op": "begin",
        "generation": GENERATION,
        "held": held_page(ctx),
    })
}

/// The `next` call's page, in the machine's own field names.
///
/// The required keys are always present (`[]`/`null` when the frame posted
/// nothing there). The optional slots — `here`, the local player's own target
/// pair, `hitpoints`, `varp95`, the board and its generation — are posted only
/// when this frame carried the fact, so the machine reads an unposted slot as
/// unobserved rather than as a default it was never handed.
fn next_payload(ctx: &ScriptCtx<'_>, token: u64) -> Value {
    let mut page = Map::new();
    page.insert("op".into(), json!("next"));
    page.insert("token".into(), json!(token));
    page.insert("generation".into(), json!(GENERATION));
    page.insert("held".into(), held_page(ctx));
    page.insert("hold".into(), json!(ctx.compiled.hold));
    page.insert("locs".into(), loc_page(ctx));
    page.insert("ground".into(), ground_page(ctx));
    page.insert("inv".into(), inv_page(ctx));
    page.insert("npcs".into(), npc_page(ctx));
    if let Some((x, z, level)) = ctx.here {
        page.insert("here".into(), json!({ "x": x, "z": z, "level": level }));
    }
    let Some(snapshot) = ctx.snapshot else {
        return Value::Object(page);
    };
    page.insert("main_modal_id".into(), json!(snapshot.modals().main));
    page.insert("inv_size".into(), json!(snapshot.inventory_size()));
    page.insert("self_slot".into(), json!(snapshot.self_slot()));
    if let Some(target) = snapshot
        .local_player()
        .and_then(|local| local.player.actor.target)
    {
        let (kind, index) = wire_target(Some(target));
        page.insert("self_target_kind".into(), json!(kind));
        page.insert("self_target_index".into(), json!(index));
    }
    if let Some(hitpoints) = snapshot
        .stats()
        .iter()
        .find(|stat| stat.name == "hitpoints")
    {
        page.insert("hitpoints".into(), json!(hitpoints.effective));
    }
    if let Some(overlay) = snapshot.varps().iter().find(|varp| varp.index == 95) {
        page.insert("varp95".into(), json!(overlay.value));
    }
    let board = snapshot.puzzle_board();
    page.insert(
        "puzzle_board".into(),
        json!({
            "component_id": board.component_id,
            "size": board.size,
            "items": board
                .items
                .iter()
                .map(|item| json!({ "slot": item.slot, "id": item.def.id }))
                .collect::<Vec<_>>(),
        }),
    );
    page.insert("puzzle_board_generation".into(), json!(board.generation));
    Value::Object(page)
}

/// The posted pack page the identify reads: `(obj id, count)` pairs in slot
/// order, and nothing else. An absent frame is an empty page, never an error.
fn held_page(ctx: &ScriptCtx<'_>) -> Value {
    let rows: &[api::snapshot::ItemView] = ctx.snapshot.map_or(&[], GameSnapshot::inventory);
    Value::Array(
        rows.iter()
            .map(|item| json!([item.def.id, item.count]))
            .collect(),
    )
}

/// The posted scene page: every placed loc on the observed frame with its
/// native ops.
fn loc_page(ctx: &ScriptCtx<'_>) -> Value {
    let rows: &[api::snapshot::LocView] = ctx.snapshot.map_or(&[], GameSnapshot::locs);
    Value::Array(
        rows.iter()
            .map(|loc| {
                json!({
                    "id": loc.id,
                    "x": loc.tile.x,
                    "z": loc.tile.z,
                    "level": loc.tile.level,
                    "actions": action_strings(&loc.actions),
                })
            })
            .collect(),
    )
}

/// The posted ground page the collect arm Takes from: the scene shape plus the
/// display name the host resolves. A row whose id resolves to no name posts
/// `null`, exactly like the posted page it is marshalled from.
fn ground_page(ctx: &ScriptCtx<'_>) -> Value {
    let rows: &[api::snapshot::GroundItemView] =
        ctx.snapshot.map_or(&[], GameSnapshot::ground_items);
    let names = ctx.obj_names;
    Value::Array(
        rows.iter()
            .map(|item| {
                json!({
                    "id": item.def.id,
                    "x": item.tile.x,
                    "z": item.tile.z,
                    "level": item.tile.level,
                    "actions": action_strings(&item.actions),
                    "name": names.and_then(|names| names.name(item.def.id)),
                })
            })
            .collect(),
    )
}

/// The posted pack page the collect arm reads: the display name the Drop
/// resolves and the positive count that occupies a slot. This is not a second
/// inventory read — the identify page above is the same observed frame.
fn inv_page(ctx: &ScriptCtx<'_>) -> Value {
    let rows: &[api::snapshot::ItemView] = ctx.snapshot.map_or(&[], GameSnapshot::inventory);
    let names = ctx.obj_names;
    Value::Array(
        rows.iter()
            .map(|item| {
                json!({
                    "id": item.def.id,
                    "name": names
                        .and_then(|names| names.name(item.def.id))
                        .or(item.def.name.as_deref()),
                    "count": item.count,
                })
            })
            .collect(),
    )
}

/// The posted npc page the guarded encounter observes: the slot index the
/// Attack rides, the type id, the posted name and tile, the health pair the
/// kill is read through, and the target pair `targets_me` compares.
fn npc_page(ctx: &ScriptCtx<'_>) -> Value {
    let rows: &[api::snapshot::NpcView] = ctx.snapshot.map_or(&[], GameSnapshot::npcs);
    Value::Array(
        rows.iter()
            .map(|npc| {
                let (target_kind, target_index) = wire_target(npc.target);
                json!({
                    "index": npc.index as i32,
                    "id": npc.r#type.map_or(-1, |kind| kind as i32),
                    "name": npc.name,
                    "x": npc.tile.x,
                    "z": npc.tile.z,
                    "level": npc.tile.level,
                    "distance": npc.distance,
                    "health": npc.health,
                    "max_health": npc.total_health,
                    "in_combat": npc.in_combat,
                    "actions": action_strings(&npc.actions),
                    "target_kind": target_kind,
                    "target_index": target_index,
                })
            })
            .collect(),
    )
}

/// A native op list as the posted page carries it: the ops that are strings,
/// in slot order.
fn action_strings(actions: &[Option<String>]) -> Vec<&str> {
    actions
        .iter()
        .filter_map(|action| action.as_deref())
        .collect()
}

/// The posted target pair: `1` an npc, `2` a player, and the `(0, -1)` pair
/// neither the machine's `targets_me` nor its `we_target` read matches — the
/// same wire pair the posted scene rows carry.
fn wire_target(target: Option<ActorTargetView>) -> (i32, i32) {
    match target {
        Some(target) => (
            match target.kind {
                ActorKind::Npc => 1,
                ActorKind::Player => 2,
            },
            target.index as i32,
        ),
        None => (0, -1),
    }
}

/// The token a `begin` answer started, when it started one.
fn answered_token(answer: &Value) -> Option<u64> {
    (answer.get("kind").and_then(Value::as_str) == Some("token"))
        .then(|| answer.get("token").and_then(Value::as_u64))
        .flatten()
}

/// Map one machine verb onto this slot's interact queue: the same variants the
/// isolate forwards for the same verbs, one explicit arm per kind.
///
/// The walk is the ordinary [`InteractReq::Walk`] with every `FindOptions` bit
/// off — never `WalkTo` (host navigation) and never a driver call. The loc,
/// npc and obj verbs keep the identity the machine posted beside them, so the
/// host matches that row and refuses a stale one. An unknown kind is not a
/// verb: nothing is enqueued for it, and a step missing a field it needs is
/// not a verb either.
fn enqueue(sink: &mut Vec<InteractReq>, kind: &str, step: &Value) {
    match kind {
        "walk" => {
            let (Some(x), Some(z), Some(level)) =
                (int_of(step, "x"), int_of(step, "z"), int_of(step, "level"))
            else {
                return;
            };
            sink.push(InteractReq::Walk {
                x,
                z,
                level,
                allow_teleports: false,
                allow_wilderness: false,
                allow_bank_fetch: false,
                request_id: 0,
            });
        }
        "held" => {
            let (Some(name), Some(action)) = (text_of(step, "name"), text_of(step, "action"))
            else {
                return;
            };
            sink.push(InteractReq::Held {
                name: name.to_string(),
                action: action.to_string(),
            });
        }
        "loc" => {
            let (Some(x), Some(z), Some(level), Some(action), Some(id)) = (
                int_of(step, "x"),
                int_of(step, "z"),
                int_of(step, "level"),
                text_of(step, "action"),
                int_of(step, "id"),
            ) else {
                return;
            };
            sink.push(InteractReq::Loc {
                x,
                z,
                level,
                action: action.to_string(),
                id: Some(id),
            });
        }
        "npc" => {
            let (Some(name), Some(action), Some(index)) = (
                text_of(step, "name"),
                text_of(step, "action"),
                int_of(step, "index"),
            ) else {
                return;
            };
            sink.push(InteractReq::Npc {
                name: name.to_string(),
                action: action.to_string(),
                index: Some(index),
            });
        }
        "if-button" => {
            let Some(component_id) = int_of(step, "component_id") else {
                return;
            };
            sink.push(InteractReq::IfButton { component_id });
        }
        "close-modal" => {
            sink.push(InteractReq::CloseModal);
        }
        "obj" => {
            let (Some(x), Some(z), Some(level), Some(name), Some(action)) = (
                int_of(step, "x"),
                int_of(step, "z"),
                int_of(step, "level"),
                text_of(step, "name"),
                text_of(step, "action"),
            ) else {
                return;
            };
            sink.push(InteractReq::Obj {
                x,
                z,
                level,
                name: Some(name.to_string()),
                action: action.to_string(),
            });
        }
        "puzzle-move" => {
            let (Some(id), Some(slot), Some(component), Some(generation)) = (
                int_of(step, "id"),
                int_of(step, "slot"),
                int_of(step, "component"),
                step.get("generation").and_then(Value::as_u64),
            ) else {
                return;
            };
            sink.push(InteractReq::PuzzleMove {
                id,
                slot,
                component,
                generation,
            });
        }
        _ => {}
    }
}

/// One integer field of a machine step.
fn int_of(step: &Value, key: &str) -> Option<i32> {
    i32::try_from(step.get(key)?.as_i64()?).ok()
}

/// One string field of a machine step.
fn text_of<'a>(step: &'a Value, key: &str) -> Option<&'a str> {
    step.get(key).and_then(Value::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::game_data::TrailMembershipRow;
    use client::client::{Client, ClientConfig};
    use client::config::if_type::{ComponentType, IfType, IfTypeMut};
    use client::config::ObjType;
    use client::io::{ClientRevision, ServerProt};
    use std::sync::Arc;

    /// The selected-revision facts every session identifies against.
    fn selected() -> Arc<SelectedGameData> {
        api::game_data::for_revision(ClientRevision::R274).expect("selected data")
    }

    /// The first selected trail row the search arm walks to: the landed
    /// `trail_loc=^true` membership with its own decodable `trail_coord`.
    fn search_row(data: &SelectedGameData) -> (i32, (i32, i32, i32)) {
        let row = data
            .trails()
            .expect("trails")
            .rows
            .iter()
            .find(|row| {
                row.role == "clue"
                    && row
                        .params
                        .iter()
                        .any(|param| param.key == "trail_loc" && param.value == "^true")
                    && coord_of(row).is_some()
            })
            .expect("a selected search row");
        (row.id, coord_of(row).expect("coord"))
    }

    /// The tile one selected `trail_coord` packs, decoded here so the test
    /// reads the walk destination off the row and not off the port:
    /// `level_mapX_mapZ_localX_localZ`, 64 tiles per map square.
    fn coord_of(row: &TrailMembershipRow) -> Option<(i32, i32, i32)> {
        let token = row
            .params
            .iter()
            .find(|param| param.key == "trail_coord")?
            .value
            .clone();
        let parts: Vec<i32> = token
            .split('_')
            .filter_map(|part| part.parse().ok())
            .collect();
        let [level, map_x, map_z, local_x, local_z] = parts[..] else {
            return None;
        };
        Some((map_x * 64 + local_x, map_z * 64 + local_z, level))
    }

    /// A membership row the selected family never names: a challenge answer
    /// id, which no identify pass reads.
    fn unrelated(data: &SelectedGameData) -> i32 {
        data.trails().expect("trails").challenge_answers[0].id
    }

    fn cfg() -> ClientConfig {
        ClientConfig {
            host: "127.0.0.1".into(),
            port: 43594,
            cache_dir: "/tmp".into(),
            members: true,
            lowmem: false,
        }
    }

    /// A client whose backpack holds exactly `held` `(obj id, count)` rows,
    /// with a named obj def per row so the posted page resolves names.
    fn client_with(held: &[(i32, i32)]) -> Client {
        let mut c = Client::new(cfg());
        if !held.is_empty() {
            let cache = Arc::get_mut(&mut c.cache).expect("sole cache owner");
            for (id, _) in held {
                while cache.objs.len() <= *id as usize {
                    cache.objs.push(ObjType::default());
                }
                cache.objs[*id as usize] = ObjType {
                    id: *id,
                    name: format!("obj {id}"),
                    ..Default::default()
                };
            }
        }
        c.side_icon[3] = 300;
        c.set_iface(
            300,
            IfType {
                id: 300,
                layer_id: 300,
                children: Some(vec![301]),
                ..Default::default()
            },
        );
        c.set_iface(
            301,
            IfType {
                id: 301,
                layer_id: 300,
                r#type: ComponentType::TYPE_INV,
                obj_ops: true,
                ..Default::default()
            },
        );
        c.set_iface_mut(
            301,
            IfTypeMut {
                link_obj_type: Some(held.iter().map(|(id, _)| id + 1).collect()),
                link_obj_number: Some(held.iter().map(|(_, count)| *count).collect()),
                ..Default::default()
            },
        );
        c.bump_gens(ServerProt::UPDATE_INV_FULL);
        c
    }

    fn snap(c: &mut Client) -> GameSnapshot {
        let mut s = GameSnapshot::new();
        s.rebuild(c);
        s
    }

    /// One observed frame: its snapshot, the posted tile and this frame's
    /// cooperative interrupt.
    struct Frame {
        snapshot: GameSnapshot,
        here: Option<(i32, i32, i32)>,
        hold: bool,
    }

    impl Frame {
        fn new(held: &[(i32, i32)], here: Option<(i32, i32, i32)>) -> Self {
            let mut c = client_with(held);
            Self {
                snapshot: snap(&mut c),
                here,
                hold: false,
            }
        }

        fn ctx<'a>(
            &'a mut self,
            driver: &'a mut dyn api::interact::Driver,
            selected: Option<&'a SelectedGameData>,
        ) -> ScriptCtx<'a> {
            ScriptCtx {
                driver,
                tick: 1,
                here: self.here,
                walk: None,
                walk_with: None,
                inv: None,
                snapshot: Some(&self.snapshot),
                obj_names: None,
                compiled: crate::CompiledTick {
                    selected,
                    hold: self.hold,
                    interacts: Some(Vec::new()),
                },
            }
        }
    }

    /// One tick over `frame`, and the requests it queued.
    fn tick(
        frame: &mut Frame,
        script: &mut Sherlock,
        selected: Option<&SelectedGameData>,
    ) -> Vec<InteractReq> {
        let mut driver = crate::ctx::test_support::NullDriver::default();
        let mut ctx = frame.ctx(&mut driver, selected);
        script.tick(&mut ctx);
        ctx.compiled.interacts.take().unwrap_or_default()
    }

    /// A tile far enough from the decoded search tile that the machine walks.
    fn far() -> (i32, i32, i32) {
        (2000, 2000, 0)
    }

    /// The one walk the machine dispatches for that search step: the ordinary
    /// `Walk` with every option the machine itself leaves off.
    fn walk_to(tile: (i32, i32, i32)) -> InteractReq {
        InteractReq::Walk {
            x: tile.0,
            z: tile.1,
            level: tile.2,
            allow_teleports: false,
            allow_wilderness: false,
            allow_bank_fetch: false,
            request_id: 0,
        }
    }

    fn page_keys(page: &Value) -> Vec<String> {
        let mut keys: Vec<String> = page
            .as_object()
            .expect("a page object")
            .keys()
            .cloned()
            .collect();
        keys.sort_unstable();
        keys
    }

    #[test]
    fn begin_page_pins_the_machine_keys() {
        let mut frame = Frame::new(&[], Some(far()));
        let mut driver = crate::ctx::test_support::NullDriver::default();
        let page = {
            let ctx = frame.ctx(&mut driver, None);
            begin_payload(&ctx)
        };
        assert_eq!(page_keys(&page), ["generation", "held", "op"]);
        assert_eq!(page["op"], "begin");
        assert_eq!(page["generation"], GENERATION);
        assert_eq!(page["held"], json!([]));
    }

    #[test]
    fn next_page_pins_the_machine_keys_over_the_observed_frame() {
        let data = selected();
        let (held_id, tile) = search_row(&data);
        let mut frame = Frame::new(&[(held_id, 1)], Some(far()));
        let mut driver = crate::ctx::test_support::NullDriver::default();
        let page = {
            let ctx = frame.ctx(&mut driver, Some(&data));
            next_payload(&ctx, 7)
        };
        assert_eq!(
            page_keys(&page),
            [
                "generation",
                "ground",
                "held",
                "here",
                "hold",
                "inv",
                "inv_size",
                "locs",
                "main_modal_id",
                "npcs",
                "op",
                "puzzle_board",
                "puzzle_board_generation",
                "self_slot",
                "token",
            ],
        );
        assert_eq!(page["op"], "next");
        assert_eq!(page["token"], 7);
        assert_eq!(page["generation"], GENERATION);
        assert_eq!(page["hold"], false);
        assert_eq!(page["held"], json!([[held_id, 1]]));
        assert_eq!(page["here"], json!({ "x": 2000, "z": 2000, "level": 0 }));
        assert_eq!(
            page["main_modal_id"], -1,
            "the observed main modal, not an invented one"
        );
        assert_eq!(page["inv_size"], 1, "the inv tab's own slot count");
        assert_eq!(page["self_slot"], -1, "the client's own local slot");
        assert_eq!(page["locs"], json!([]));
        assert_eq!(page["npcs"], json!([]));
        assert_eq!(page["ground"], json!([]));
        assert_eq!(
            page["puzzle_board"],
            json!({ "component_id": -1, "size": 0, "items": [] }),
            "a closed board is what the observed frame carries"
        );
        assert!(
            page.get("hitpoints").is_none()
                && page.get("varp95").is_none()
                && page.get("self_target_kind").is_none(),
            "a fact this frame did not carry is omitted, never defaulted: {page}"
        );
        assert_ne!(tile.0, 0, "the decoded search tile is a real destination");
    }

    #[test]
    fn next_page_omits_the_tile_and_the_pages_the_frame_lacks() {
        // No observed tile: `here` is not posted, and the required pages are
        // still the frame's own.
        let mut untiled = Frame::new(&[], None);
        let mut driver = crate::ctx::test_support::NullDriver::default();
        let untiled_page = {
            let ctx = untiled.ctx(&mut driver, None);
            next_payload(&ctx, 7)
        };
        assert!(untiled_page.get("here").is_none(), "{untiled_page}");
        assert_eq!(untiled_page["held"], json!([]));
        assert_eq!(untiled_page["hold"], false);

        // No snapshot at all: every page is empty and every optional slot is
        // omitted — never a closed `-1`, never the client's `28`, never
        // slot `0`.
        let mut unobserved = Frame::new(&[], None);
        let mut driver = crate::ctx::test_support::NullDriver::default();
        let mut ctx = unobserved.ctx(&mut driver, None);
        ctx.snapshot = None;
        let page = next_payload(&ctx, 1);
        assert_eq!(
            page_keys(&page),
            [
                "generation",
                "ground",
                "held",
                "hold",
                "inv",
                "locs",
                "npcs",
                "op",
                "token",
            ],
        );
        assert_eq!(page["held"], json!([]));
        assert_eq!(page["inv"], json!([]));
        assert_eq!(page["locs"], json!([]));
        assert_eq!(page["npcs"], json!([]));
        assert_eq!(page["ground"], json!([]));
        assert!(page.get("inv_size").is_none(), "{page}");
        assert!(page.get("main_modal_id").is_none(), "{page}");
        assert!(page.get("self_slot").is_none(), "{page}");
        assert!(page.get("puzzle_board").is_none(), "{page}");
    }

    #[test]
    fn a_held_search_row_walks_with_every_walk_option_off() {
        let data = selected();
        let (held_id, tile) = search_row(&data);
        let mut frame = Frame::new(&[(held_id, 1)], Some(far()));
        let mut script = Sherlock::default();
        let sink = tick(&mut frame, &mut script, Some(&data));
        // Reaching the walk at all is the gate answered `true`: an unanswered
        // `callback.enabled` loops on the same question and never reaches a
        // verb, so this one enqueue is the whole handoff walked through.
        assert_eq!(
            sink,
            vec![walk_to(tile)],
            "the decoded search tile, on the ordinary Walk with every option off"
        );
        assert!(script.token.is_some(), "the begin started this session");
        assert_eq!(script.solved, 0);
    }

    #[test]
    fn nothing_held_is_an_idle_begin_the_next_frame_starts() {
        let data = selected();
        let mut script = Sherlock::default();

        // Nothing held: the begin is refused, so the card stays idle and
        // enqueues nothing — this tick and the next.
        let mut bare = Frame::new(&[], Some(far()));
        for _ in 0..2 {
            assert!(tick(&mut bare, &mut script, Some(&data)).is_empty());
        }
        assert!(script.token.is_none());
        assert_eq!(script.solved, 0, "a refused begin is not a finished clue");

        // Nothing held against a pin this build does not have: the same idle.
        let mut unpinned = Frame::new(&[], Some(far()));
        assert!(tick(&mut unpinned, &mut script, None).is_empty());
        assert!(script.token.is_none());

        // The same card, one held search row: this frame begins the session
        // the idle frames waited for, so they never wedged it.
        let (held_id, tile) = search_row(&data);
        let mut held = Frame::new(&[(held_id, 1)], Some(far()));
        assert_eq!(
            tick(&mut held, &mut script, Some(&data)),
            vec![walk_to(tile)]
        );
        assert!(script.token.is_some());
    }

    #[test]
    fn the_identify_is_the_machines_own_membership_read() {
        let data = selected();
        let mut script = Sherlock::default();

        // A held id the selected family never names is not a step: this card
        // identifies nothing of its own and stays idle.
        let mut unrelated_frame = Frame::new(&[(unrelated(&data), 1)], Some(far()));
        assert!(tick(&mut unrelated_frame, &mut script, Some(&data)).is_empty());
        assert!(script.token.is_none());

        // A held row with no positive count is not held either.
        let (held_id, _) = search_row(&data);
        let mut zero = Frame::new(&[(held_id, 0)], Some(far()));
        assert!(tick(&mut zero, &mut script, Some(&data)).is_empty());
        assert!(script.token.is_none());
    }

    #[test]
    fn a_session_end_counts_the_clue_locally_and_starts_fresh_next_frame() {
        let data = selected();
        let (held_id, tile) = search_row(&data);
        let mut script = Sherlock::default();

        let mut held = Frame::new(&[(held_id, 1)], Some(far()));
        assert_eq!(
            tick(&mut held, &mut script, Some(&data)),
            vec![walk_to(tile)]
        );
        let token = script.token.expect("a live session");

        // The clue is gone from the pack: the machine's own `none-held` ends
        // the session. The card counts it locally and emits nothing — no
        // verb, no completion kind, no `clue solved`.
        let mut dropped = Frame::new(&[], Some(far()));
        assert!(tick(&mut dropped, &mut script, Some(&data)).is_empty());
        assert!(script.token.is_none(), "the session ended");
        assert_eq!(script.solved, 1);

        // A fresh frame starts a fresh session, not the old token.
        let mut again = Frame::new(&[(held_id, 1)], Some(far()));
        assert_eq!(
            tick(&mut again, &mut script, Some(&data)),
            vec![walk_to(tile)]
        );
        assert_ne!(script.token, Some(token));

        // A token the machine no longer has is the same end, without a count.
        script.token = Some(0);
        assert!(tick(&mut again, &mut script, Some(&data)).is_empty());
        assert!(script.token.is_none(), "a stale token is not a session");
        assert_eq!(script.solved, 1, "only the trail end counts");
    }

    #[test]
    fn the_page_marshals_the_observed_pack_and_scene() {
        let data = selected();
        let (held_id, _) = search_row(&data);
        let mut frame = Frame::new(&[(held_id, 1)], Some(far()));
        let mut driver = crate::ctx::test_support::NullDriver::default();
        let ctx = frame.ctx(&mut driver, Some(&data));
        let page = next_payload(&ctx, 3);
        let inv = page["inv"].as_array().expect("inv page");
        assert_eq!(inv.len(), 1, "{page}");
        assert_eq!(inv[0]["id"], held_id);
        assert_eq!(inv[0]["count"], 1);
        assert_eq!(inv[0]["name"], format!("obj {held_id}"));
        assert_eq!(
            page["held"],
            json!([[held_id, 1]]),
            "the identify page is the same observed pack"
        );
        assert!(
            page["locs"].as_array().expect("locs").is_empty(),
            "an unobserved scene is an empty page, never an invented row"
        );
    }

    #[test]
    fn the_page_carries_the_frames_cooperative_interrupt() {
        let mut frame = Frame::new(&[], Some(far()));
        let mut driver = crate::ctx::test_support::NullDriver::default();
        let inert = {
            let ctx = frame.ctx(&mut driver, None);
            next_payload(&ctx, 1)
        };
        assert_eq!(inert["hold"], false);

        // The frame's own `hold || ours`: the same pair the isolate's
        // `EventSignal.pending()` reads, so the machine yields instead of
        // driving the step on a frame the guardian froze.
        frame.hold = true;
        let held = {
            let ctx = frame.ctx(&mut driver, None);
            next_payload(&ctx, 1)
        };
        assert_eq!(held["hold"], true);
    }

    #[test]
    fn every_machine_verb_maps_onto_one_interact_request() {
        let mut sink = Vec::new();
        enqueue(
            &mut sink,
            "held",
            &json!({ "kind": "held", "name": "Spade", "action": "Dig" }),
        );
        enqueue(
            &mut sink,
            "loc",
            &json!({
                "kind": "loc",
                "x": 1,
                "z": 2,
                "level": 0,
                "action": "Search",
                "id": 2646,
            }),
        );
        enqueue(
            &mut sink,
            "npc",
            &json!({
                "kind": "npc",
                "name": "Zamorak wizard",
                "action": "Attack",
                "index": 4,
            }),
        );
        enqueue(
            &mut sink,
            "if-button",
            &json!({ "kind": "if-button", "component_id": 77 }),
        );
        enqueue(&mut sink, "close-modal", &json!({ "kind": "close-modal" }));
        enqueue(
            &mut sink,
            "obj",
            &json!({
                "kind": "obj",
                "x": 9,
                "z": 8,
                "level": 1,
                "name": "Reward casket (easy)",
                "action": "Take",
            }),
        );
        enqueue(
            &mut sink,
            "puzzle-move",
            &json!({
                "kind": "puzzle-move",
                "id": 1,
                "slot": 2,
                "component": 6600,
                "generation": 7,
            }),
        );
        assert_eq!(
            sink,
            vec![
                InteractReq::Held {
                    name: "Spade".into(),
                    action: "Dig".into(),
                },
                InteractReq::Loc {
                    x: 1,
                    z: 2,
                    level: 0,
                    action: "Search".into(),
                    id: Some(2646),
                },
                InteractReq::Npc {
                    name: "Zamorak wizard".into(),
                    action: "Attack".into(),
                    index: Some(4),
                },
                InteractReq::IfButton { component_id: 77 },
                InteractReq::CloseModal,
                InteractReq::Obj {
                    x: 9,
                    z: 8,
                    level: 1,
                    name: Some("Reward casket (easy)".into()),
                    action: "Take".into(),
                },
                InteractReq::PuzzleMove {
                    id: 1,
                    slot: 2,
                    component: 6600,
                    generation: 7,
                },
            ]
        );

        // A kind that is not a verb, and a verb missing a field it needs:
        // neither is enqueued as anything.
        let mut unknown = Vec::new();
        enqueue(&mut unknown, "done", &json!({ "kind": "done" }));
        enqueue(&mut unknown, "walk", &json!({ "kind": "walk", "x": 1 }));
        assert!(
            unknown.is_empty(),
            "a kind that is not a verb, and a verb with no fields, enqueue nothing"
        );
    }

    #[test]
    fn a_tick_without_a_sink_dispatches_nothing() {
        let data = selected();
        let (held_id, _) = search_row(&data);
        let mut frame = Frame::new(&[(held_id, 1)], Some(far()));
        let mut script = Sherlock::default();
        let mut driver = crate::ctx::test_support::NullDriver::default();
        {
            let mut ctx = frame.ctx(&mut driver, Some(&data));
            ctx.compiled.interacts = None;
            script.tick(&mut ctx);
        }
        assert!(
            script.token.is_none(),
            "a card with nowhere to send a verb does not start a session"
        );
    }

    #[test]
    fn a_frozen_frame_yields_instead_of_walking() {
        let data = selected();
        let (held_id, _) = search_row(&data);
        let mut frame = Frame::new(&[(held_id, 1)], Some(far()));
        frame.hold = true;
        let mut script = Sherlock::default();
        // The guardian's own freeze: the machine yields the step rather than
        // walking through the held frame. Nothing is enqueued, and the token
        // it began with stays live for the thaw.
        let sink = tick(&mut frame, &mut script, Some(&data));
        assert!(sink.is_empty(), "{sink:?}");
        assert!(
            script.token.is_some(),
            "the frozen frame is not a session end"
        );
    }
}
