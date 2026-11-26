use std::collections::HashMap;
use std::sync::Arc;

use api::bank_memory::BankMemory;
use api::host_log;
use api::hostlog::{Category, Level};
use api::interact::Driver;
use api::snapshot::{GameSnapshot, WorldTile};
use client::config::Cache;
use nav_planner::router::FindOptions;
use nav_planner::world::NavWorld;
use nav_planner::WorldState;
use parking_lot::RwLock;

use super::{route_inspect, NavBot, ScriptRouteExclusions, ScriptWalkArm};
use crate::catalog_core::ScriptAct;
#[cfg(feature = "memory-profile")]
use crate::memory_diagnostics;
/// The host's fail-closed accept gate for a clue duel `DuelAccept`: the
/// slot's own selected duel `controls`, the exact screen, no stake on
/// either side, obstacles-only rules, and the partner named on the offer
/// screen (`offer_partner` holds it, per slot, until the confirm).
pub(crate) fn validated_duel_accept(
    snapshot: &GameSnapshot,
    controls: &api::game_data::DuelControls,
    offer_partner: &mut Option<String>,
    screen: &str,
    partner: &str,
    rules: i32,
) -> Option<i32> {
    if rules != 1024 {
        return None;
    }
    let (root, mine, theirs, accept) = match screen {
        "offer" => (
            controls.select_modal,
            controls.select_mine,
            controls.select_theirs,
            controls.select_accept,
        ),
        "confirm" => (
            controls.confirm_modal,
            controls.confirm_mine,
            controls.confirm_theirs,
            controls.confirm_accept,
        ),
        _ => return None,
    };
    if snapshot.modals().main != root {
        return None;
    }
    let empty = |component_id| {
        snapshot
            .widgets()
            .iter()
            .find(|widget| widget.component_id == component_id && widget.type_ == 2)
            .map(|widget| widget.items.is_empty())
    };
    let mine_empty = empty(mine);
    let theirs_empty = empty(theirs);
    if (screen == "offer" && (mine_empty != Some(true) || theirs_empty != Some(true)))
        || mine_empty == Some(false)
        || theirs_empty == Some(false)
    {
        return None;
    }
    if snapshot
        .varps()
        .iter()
        .find(|varp| varp.index == controls.options_varp)
        .map(|varp| varp.value)
        != Some(rules)
    {
        return None;
    }
    let expected = script::duel_name(partner);
    if screen == "offer" {
        let actual = snapshot
            .widgets()
            .iter()
            .find(|widget| widget.component_id == controls.select_partner)
            .and_then(|widget| widget.text.as_deref())
            .and_then(script::duel_partner_name)
            .map(|name| script::duel_name(&name));
        if actual.as_deref() != Some(expected.as_str()) {
            return None;
        }
        *offer_partner = Some(expected);
    } else if offer_partner.as_deref() != Some(expected.as_str()) {
        return None;
    }
    Some(accept)
}
fn act_tile(x: i32, z: i32, level: i32) -> crate::catalog_core::LineOfSightTile {
    crate::catalog_core::LineOfSightTile { x, z, level }
}

fn route_exclusions(
    avoid: Vec<script::interact::InspectAvoidWire>,
    cross: Vec<String>,
) -> ScriptRouteExclusions {
    ScriptRouteExclusions {
        avoid_wire: avoid,
        cross: cross.into_iter().map(Arc::<str>::from).collect(),
        ..ScriptRouteExclusions::default()
    }
}

/// Preserve a live compat owner until the replacement is accepted. The host
/// publishes one outer terminal before it releases that owner's seat.
pub(crate) fn retire_logical_walk(
    slot: Option<&mut script::SlotScript>,
    navs: &Arc<crate::SharedMutex<HashMap<String, NavBot>>>,
    name: &str,
    request_id: Option<u64>,
    end: script::native::WalkEnd,
) -> bool {
    if request_id == Some(0) {
        return false;
    }
    let mut navs = navs.lock().unwrap();
    let Some(bot) = navs.get_mut(name) else {
        return false;
    };
    let compat_id = match request_id {
        Some(request_id) => bot
            .compat_walk_request_id()
            .filter(|live| *live == request_id),
        None => bot.compat_walk_request_id(),
    };
    if let Some(request_id) = compat_id {
        let Some(slot) = slot else {
            return false;
        };
        slot.abort_compat_walk(request_id);
        if let Some((run, terminal_request_id, result)) = slot.take_compat_walk_terminal() {
            if terminal_request_id.get() != request_id || bot.compat_walk_run() != Some(run) {
                return false;
            }
            let (terminal_end, detail) = match result {
                Ok(receipt) => {
                    let detail = receipt.failure_detail();
                    (receipt.end, detail)
                }
                Err(error) => {
                    let terminal_end = match &error {
                        script::native::ActionError::Cancelled => {
                            script::native::WalkEnd::Cancelled
                        }
                        script::native::ActionError::UserInput => {
                            script::native::WalkEnd::UserInput
                        }
                        _ => script::native::WalkEnd::Failed,
                    };
                    (terminal_end, Some(Arc::from(error.to_string())))
                }
            };
            if !bot.complete_compat_walk(request_id, terminal_end, detail) {
                return false;
            }
            slot.finish_compat_walk(request_id);
            return true;
        }
        if !bot.complete_compat_walk(request_id, end, None) {
            return false;
        }
        slot.finish_compat_walk(request_id);
        return true;
    }
    if request_id.is_some_and(|request_id| bot.walk_request_id != request_id) {
        return false;
    }
    super::script_walk::abort_walk_on_bot_with_end(bot, end);
    true
}

/// Same compat identity and route after a reconnect carry was re-armed.
fn already_following(
    navs: &Arc<crate::SharedMutex<HashMap<String, NavBot>>>,
    name: &str,
    request_id: u64,
    key: (WorldTile, i32, bool, bool, bool),
    exclusions: &ScriptRouteExclusions,
) -> bool {
    request_id != 0
        && navs
            .lock()
            .unwrap()
            .get(name)
            .is_some_and(|bot| bot.compat_walk_is_following(request_id, key, exclusions))
}

/// Note one game request host-play dispatched for `slot`'s script. The
/// catalog hunt watch reads this record; the isolate never sees it.
fn record_script_act(
    navs: &Arc<crate::SharedMutex<HashMap<String, NavBot>>>,
    slot: &str,
    act: ScriptAct,
) {
    let mut navs = navs.lock().unwrap();
    match navs.get_mut(slot) {
        Some(bot) => bot.acts.record(act),
        None => navs.entry(slot.to_string()).or_default().acts.record(act),
    }
}

/// Dispatch one isolate's shim interact requests. Open/close/deposit/
/// withdraw run through [`api::interact::Interactions`] on the slot's
/// snapshot + Driver — a request whose target is missing (no loc at the
/// tile, no bank-side row with the resolved name, no bank open) fails
/// closed with no send. `walk` / `walk-near` route through the shared
/// [`ScriptWalkArm`] with the request's three FindOptions bits (serde/old
/// wire default off). `walk-to` (scene `DirectNavigator`) is the
/// [`Interactions::walk_nearest`] packet, matching frozen
/// `ClientAdapter.walkTo(... tryNearest=true)`. Returns whether the driver's
/// out buffer was written. The test seam: the account's bank memory is what
/// the slot's observer leaves this frame (the loaded open bank, else an
/// unobserved `Unknown` bank).
#[cfg(test)]
#[expect(
    clippy::too_many_arguments,
    reason = "the test wrapper supplies the same observed inputs as scoped interaction dispatch"
)]
pub(crate) fn dispatch_script_interact(
    driver: &mut dyn Driver,
    snapshot: &GameSnapshot,
    obj_names: Option<&api::obj_names::ObjNames>,
    here: Option<(i32, i32, i32)>,
    navs: &Arc<crate::SharedMutex<HashMap<String, NavBot>>>,
    world: &Option<Arc<NavWorld>>,
    state: Option<WorldState>,
    name: &str,
    reqs: Vec<script::interact::InteractReq>,
) -> bool {
    let bank = RwLock::new(BankMemory::default());
    bank.write().track(snapshot, 0);
    let state = state.map(Arc::new);
    dispatch_script_interact_cached_scoped(
        driver,
        &InteractContext {
            snapshot,
            obj_names,
            here,
            navs,
            world,
            state: state.as_ref(),
            bank: Some(&bank),
            name,
            compatibility_scope: Some(api::identity::Scope::Slot(
                api::identity::SlotKey::from_incarnation(1).expect("test slot is bound"),
            )),
            cache: &None,
            obj_names_arc: &None,
        },
        None,
        reqs,
    )
}

pub(crate) struct BankDepositDispatch {
    pub accepted: bool,
    pub refusal: Option<String>,
}

pub(crate) fn dispatch_native_bank_deposit(
    driver: &mut dyn Driver,
    snapshot: &GameSnapshot,
    trace: &script::native::BankDepositTrace,
) -> BankDepositDispatch {
    use api::interact::{ActionSpec, OpTarget, SendResult};

    let refuse = |reason: &str| BankDepositDispatch {
        accepted: false,
        refusal: Some(reason.to_owned()),
    };
    if snapshot.bank_component_id() < 0 {
        return refuse("bank-not-open");
    }
    if !snapshot.bank_loaded() {
        return refuse("bank-not-loaded");
    }
    if snapshot.bank_session_generation() != trace.generation {
        return refuse("generation-mismatch");
    }
    let Some(item) = snapshot.bank_side().iter().find(|item| {
        item.def.id == trace.id && item.slot == trace.slot && item.component_id == trace.component
    }) else {
        return refuse("row-missing");
    };
    let option = usize::try_from(trace.operation)
        .ok()
        .and_then(|operation| operation.checked_sub(1))
        .and_then(|index| item.actions.get(index))
        .and_then(Option::as_deref);
    if option != Some(trace.option.as_ref()) {
        return refuse("operation-mismatch");
    }
    match api::interact::Interactions::new(snapshot, driver)
        .interact(OpTarget::Item(item), ActionSpec::Operation(trace.operation))
    {
        SendResult::Sent { .. } => BankDepositDispatch {
            accepted: true,
            refusal: None,
        },
        SendResult::Refused { reason, .. } => refuse(&format!("{reason:?}")),
    }
}

/// Borrowed authority-independent inputs for one exclusive ordered drain.
pub(crate) struct OrderedDispatchContext<'a> {
    pub snapshot: &'a GameSnapshot,
    pub selected: Option<&'a api::game_data::SelectedGameData>,
    pub input: Option<&'a host::SlotInput>,
    /// Derived by the host from the exact active compiled Gatherer run, never the payload.
    pub builtin_gatherer_knife: bool,
}

/// Dispatch both sealed modes through the same API planner and encrypted sink.
pub(crate) fn dispatch_native_server_ordered(
    driver: &mut dyn Driver,
    context: OrderedDispatchContext<'_>,
    authority: &script::native::HostAuthority,
    envelope: &script::surface::ServerOrderedEnvelope,
) -> api::interact::server_ordered::ServerOrderedDispatch {
    use api::interact::server_ordered::{
        InputLineage, ServerOrderedDispatch, ServerOrderedDispatchContext, ServerOrderedError,
        ServerOrderedInputs,
    };
    use script::surface::ServerOrderedPayload;

    // Dispatch uses the raw Client PLAYER_INFO generation shared by the snapshot
    // and input observer. The session-relative snapshot tick and Play's native
    // completion tick are separate clocks, including after relogs.
    let dispatch_tick = context.snapshot.gens().player_info;
    let request_id = authority.request_id().get();
    let row_count = match envelope.payload() {
        ServerOrderedPayload::Content(request) => request.row_count(),
        ServerOrderedPayload::Advanced(operations) => operations.len() as u8,
    };
    let Some(input) = context.input else {
        return ServerOrderedDispatch::refused(
            request_id,
            dispatch_tick,
            row_count,
            0,
            ServerOrderedError::ObservationUnavailable,
        );
    };
    // A legacy encoder can leave its last variable-length packet open. Observe
    // it under its original lineage before capturing this request's input proof.
    driver.finish_pending_packet();
    let observed = input.with_server_ordered_input(|state| state.snapshot());
    let mut live = || {
        authority
            .live()
            .then_some(())
            .ok_or(ServerOrderedError::Cancelled)
    };
    match envelope.payload() {
        ServerOrderedPayload::Content(request) => {
            let row_count = request.row_count();
            let Some(selected) = context.selected else {
                return ServerOrderedDispatch::refused(
                    request_id,
                    dispatch_tick,
                    row_count,
                    0,
                    ServerOrderedError::ContractUnavailable,
                );
            };
            let Ok(catalog) = selected.server_ordered_catalog() else {
                return ServerOrderedDispatch::refused(
                    request_id,
                    dispatch_tick,
                    row_count,
                    0,
                    ServerOrderedError::ContractUnavailable,
                );
            };
            let _lineage = input.input_lineage(InputLineage::ContentRequest(request_id));
            let dispatch_context = ServerOrderedDispatchContext {
                request_id,
                dispatch_tick,
                input: observed,
                debug: None,
                authority: &mut live,
            };
            if context.builtin_gatherer_knife
                && matches!(
                    request,
                    api::interact::server_ordered::ServerOrderedRequest::KnifeArm { .. }
                )
            {
                ServerOrderedInputs::dispatch_builtin_gatherer_knife(
                    catalog,
                    driver.revision(),
                    context.snapshot,
                    driver,
                    request,
                    dispatch_context,
                )
            } else {
                ServerOrderedInputs::dispatch(
                    catalog,
                    driver.revision(),
                    context.snapshot,
                    driver,
                    request,
                    dispatch_context,
                )
            }
        }
        ServerOrderedPayload::Advanced(operations) => {
            let _lineage = input.input_lineage(InputLineage::AdvancedRequest(request_id));
            ServerOrderedInputs::dispatch_advanced(
                context.snapshot,
                driver,
                operations,
                ServerOrderedDispatchContext {
                    request_id,
                    dispatch_tick,
                    input: observed,
                    debug: envelope.debug_catalog(),
                    authority: &mut live,
                },
            )
        }
    }
}

#[cfg(test)]
#[expect(
    clippy::too_many_arguments,
    reason = "the test wrapper preserves cached inputs without constructing a live slot"
)]
pub(crate) fn dispatch_script_interact_cached<R>(
    driver: &mut dyn Driver,
    snapshot: &GameSnapshot,
    obj_names: Option<&api::obj_names::ObjNames>,
    here: Option<(i32, i32, i32)>,
    navs: &Arc<crate::SharedMutex<HashMap<String, NavBot>>>,
    world: &Option<Arc<NavWorld>>,
    state: Option<WorldState>,
    bank: Option<&RwLock<BankMemory>>,
    name: &str,
    reqs: R,
    cache: Option<Arc<Cache>>,
    obj_names_arc: Option<Arc<api::obj_names::ObjNames>>,
) -> bool
where
    R: AsRef<[script::interact::InteractReq]> + IntoIterator<Item = script::interact::InteractReq>,
{
    let state = state.map(Arc::new);
    dispatch_script_interact_cached_scoped(
        driver,
        &InteractContext {
            snapshot,
            obj_names,
            here,
            navs,
            world,
            state: state.as_ref(),
            bank,
            name,
            compatibility_scope: None,
            cache: &cache,
            obj_names_arc: &obj_names_arc,
        },
        None,
        reqs,
    )
}

/// The slot context at one interaction dispatch boundary. Dispatches borrow the
/// same frame's gating facts; the snapshot borrow is confined to that dispatch
/// so test-only ordered hooks can mutate it between requests. The driver stays
/// separate because the pass also writes through it. Only an arm that
/// queues work takes a shared `Arc` handle at the event boundary.
#[derive(Clone, Copy)]
pub(crate) struct InteractContext<'a> {
    pub(crate) snapshot: &'a GameSnapshot,
    pub(crate) obj_names: Option<&'a api::obj_names::ObjNames>,
    pub(crate) here: Option<(i32, i32, i32)>,
    pub(crate) navs: &'a Arc<crate::SharedMutex<HashMap<String, NavBot>>>,
    pub(crate) world: &'a Option<Arc<NavWorld>>,
    pub(crate) state: Option<&'a Arc<WorldState>>,
    pub(crate) bank: Option<&'a RwLock<BankMemory>>,
    pub(crate) name: &'a str,
    pub(crate) compatibility_scope: Option<api::identity::Scope>,
    pub(crate) cache: &'a Option<Arc<Cache>>,
    pub(crate) obj_names_arc: &'a Option<Arc<api::obj_names::ObjNames>>,
}

/// What one interaction dispatch learned beyond whether it wrote. The native
/// single-request path reads it; a multi-request dispatch keeps the last
/// gated world operation's.
#[derive(Default)]
pub(crate) struct InteractOutcome {
    pub(crate) wrote: bool,
    /// The reach class of the last gated world operation. `Unreachable` means
    /// the owned path refused it and wrote nothing.
    pub(crate) reach: Option<api::interact::OpReach>,
    /// The stall watch an accepted world operation armed.
    pub(crate) watch: Option<api::interact::ApproachWatch>,
}

impl InteractOutcome {
    /// Take the previous request's reach facts out of `ix`.
    fn absorb(&mut self, ix: &mut api::interact::Interactions<'_>, slot: &str) {
        let Some(reach) = ix.last_reach() else {
            return;
        };
        self.reach = Some(reach);
        self.watch = ix.take_approach_watch();
        ix.clear_reach();
        if reach == api::interact::OpReach::Unreachable {
            host_log!(
                Category::InteractTrace,
                Level::Debug,
                slot = slot,
                "world op refused: no route reaches the target and no approach script can serve it"
            );
        }
    }
}

pub(crate) fn dispatch_script_interact_cached_scoped<R>(
    driver: &mut dyn Driver,
    context: &InteractContext<'_>,
    owner_slot: Option<&mut script::SlotScript>,
    reqs: R,
) -> bool
where
    R: AsRef<[script::interact::InteractReq]> + IntoIterator<Item = script::interact::InteractReq>,
{
    dispatch_script_interact_outcome(driver, context, owner_slot, reqs).wrote
}

pub(crate) fn dispatch_script_interact_outcome<R>(
    driver: &mut dyn Driver,
    context: &InteractContext<'_>,
    mut owner_slot: Option<&mut script::SlotScript>,
    reqs: R,
) -> InteractOutcome
where
    R: AsRef<[script::interact::InteractReq]> + IntoIterator<Item = script::interact::InteractReq>,
{
    let mut outcome = InteractOutcome::default();
    use script::interact::InteractReq;
    let InteractContext {
        snapshot,
        navs,
        name,
        ..
    } = *context;
    #[cfg(feature = "memory-profile")]
    memory_diagnostics::requests(name, reqs.as_ref());
    let mut wrote = false;
    let camera_yaws: Vec<i32> = reqs
        .as_ref()
        .iter()
        .filter_map(|req| match req {
            InteractReq::SetCameraYaw { yaw } => Some(*yaw),
            _ => None,
        })
        .collect();
    // The slot's own session revision selects the duel controls it accepts.
    let revision = driver.revision();
    let mut ix = api::interact::Interactions::new(snapshot, driver);
    if api::hostlog::enabled(Category::InteractTrace) {
        for req in reqs.as_ref() {
            host_log!(
                Category::InteractTrace,
                Level::Debug,
                slot = name,
                "interact {req:?}"
            );
        }
    }
    for req in reqs.as_ref() {
        if let InteractReq::InspectAck { seq, generation } = req {
            if let Some(bot) = navs.lock().unwrap().get_mut(name) {
                bot.inspect.apply_ack(*seq, *generation);
            }
        }
    }
    for req in reqs {
        outcome.absorb(&mut ix, name);
        wrote |= dispatch_single_interact_req(
            &mut ix,
            context,
            owner_slot.as_deref_mut(),
            revision,
            req,
        );
    }
    outcome.absorb(&mut ix, name);
    for yaw in camera_yaws {
        wrote |= driver.set_orbit_camera_yaw(yaw);
    }
    #[cfg(feature = "memory-profile")]
    memory_diagnostics::sent(name, wrote);
    outcome.wrote = wrote;
    outcome
}

fn dispatch_single_interact_req(
    ix: &mut api::interact::Interactions<'_>,
    context: &InteractContext<'_>,
    owner_slot: Option<&mut script::SlotScript>,
    revision: api::selected::ClientRevision,
    req: script::interact::InteractReq,
) -> bool {
    use api::interact::SendResult;
    use script::interact::InteractReq;
    match req {
        InteractReq::OpenBooth {
            x,
            z,
            level,
            id,
            name: booth_name,
            action,
        } => dispatch_open_booth(
            ix,
            context,
            owner_slot,
            WorldTile { x, z, level },
            id,
            booth_name.as_deref(),
            action.as_deref(),
        ),
        InteractReq::OpenStand {
            x,
            z,
            level,
            kind,
            name,
            stand_op,
            ..
        } => dispatch_open_stand(
            ix,
            context,
            owner_slot,
            WorldTile { x, z, level },
            &kind,
            name.as_deref(),
            stand_op,
        ),
        InteractReq::Walk {
            x,
            z,
            level,
            allow_teleports,
            allow_wilderness,
            allow_bank_fetch,
            allow_prereq_dialogue,
            allow_quest_start,
            request_id,
            track_door_crossing,
            avoid,
            cross,
        } => dispatch_walk_request(
            context,
            owner_slot,
            WalkRequestArgs {
                dest: WorldTile { x, z, level },
                radius: 0,
                exact: true,
                compat_tile_intent: false,
                allow_teleports,
                allow_wilderness,
                allow_bank_fetch,
                allow_prereq_dialogue,
                allow_quest_start,
                request_id,
                track_door_crossing,
                avoid,
                cross,
            },
        ),
        InteractReq::WalkNear {
            x,
            z,
            level,
            radius,
            compat_tile_intent,
            allow_teleports,
            allow_wilderness,
            allow_bank_fetch,
            allow_prereq_dialogue,
            allow_quest_start,
            request_id,
            track_door_crossing,
            avoid,
            cross,
        } => dispatch_walk_request(
            context,
            owner_slot,
            WalkRequestArgs {
                dest: WorldTile { x, z, level },
                radius,
                exact: false,
                compat_tile_intent,
                allow_teleports,
                allow_wilderness,
                allow_bank_fetch,
                allow_prereq_dialogue,
                allow_quest_start,
                request_id,
                track_door_crossing,
                avoid,
                cross,
            },
        ),
        // Not a game packet: the follow stops, nothing is written.
        InteractReq::AbortWalk { request_id } => {
            retire_logical_walk(
                owner_slot,
                context.navs,
                context.name,
                Some(request_id),
                script::native::WalkEnd::Cancelled,
            );
            false
        }
        InteractReq::WalkNearestBank => dispatch_walk_nearest_bank(context),
        InteractReq::WalkTo { x, z, level } => {
            dispatch_walk_to(ix, context, WorldTile { x, z, level })
        }
        InteractReq::InspectRoute {
            x,
            z,
            level,
            from_x,
            from_z,
            from_level,
            allow_teleports,
            allow_wilderness,
            allow_bank_fetch,
            avoid,
            cross,
            request_id,
            max_expansions,
        } => {
            dispatch_inspect_route(
                context,
                InspectRouteArgs {
                    from: WorldTile {
                        x: from_x,
                        z: from_z,
                        level: from_level,
                    },
                    to: WorldTile { x, z, level },
                    allow_teleports,
                    allow_wilderness,
                    allow_bank_fetch,
                    avoid,
                    cross,
                    request_id,
                    max_expansions,
                },
            );
            true
        }
        InteractReq::InspectAck { seq, generation } => {
            if let Some(bot) = context.navs.lock().unwrap().get_mut(context.name) {
                bot.inspect.apply_ack(seq, generation);
            }
            false
        }
        InteractReq::Deposit { name, id } => dispatch_deposit(ix, context, &name, id),
        InteractReq::Withdraw { name, action } => {
            dispatch_withdraw(ix, context.snapshot, context.obj_names, &name, &action)
        }
        // Direct callers cannot safely create host pending state.
        InteractReq::WithdrawX { .. } => false,
        InteractReq::Held {
            name,
            action,
            slot: item_slot,
            target_item_id,
        } => dispatch_held(
            ix,
            context.snapshot,
            context.obj_names,
            &name,
            &action,
            item_slot,
            target_item_id,
        ),
        InteractReq::InvButton {
            id,
            slot: item_slot,
            component,
            operation,
            bank_generation,
        } => dispatch_inv_button(
            ix,
            context.snapshot,
            id,
            item_slot,
            component,
            operation,
            bank_generation,
        ),
        InteractReq::BankDrag {
            id,
            slot: item_slot,
            component,
            target_slot,
            insert,
            bank_generation,
        } => dispatch_bank_drag(
            ix,
            context.snapshot,
            BankDragArgs {
                id,
                item_slot,
                component,
                target_slot,
                insert,
                bank_generation,
            },
        ),
        InteractReq::PuzzleMove {
            id,
            slot: item_slot,
            component,
            generation,
        } => dispatch_puzzle_move(
            ix,
            context.snapshot,
            context.cache.as_deref(),
            id,
            item_slot,
            component,
            generation,
        ),
        InteractReq::ShopButton {
            kind,
            name,
            id,
            slot: item_slot,
            component,
            chunk,
        } => dispatch_shop_button(
            ix,
            context.snapshot,
            ShopButtonArgs {
                kind: &kind,
                name: &name,
                id,
                item_slot,
                component,
                chunk,
            },
        ),
        InteractReq::MakePanel {
            id,
            slot: item_slot,
            component,
            operation,
        } => dispatch_make_panel(ix, context.snapshot, id, item_slot, component, operation),
        InteractReq::Close => dispatch_close_logged(ix),
        InteractReq::Npc {
            name,
            action,
            index,
        } => dispatch_npc(ix, context, &name, action, index),
        InteractReq::Loc {
            x,
            z,
            level,
            action,
            id,
        } => dispatch_loc(ix, context, WorldTile { x, z, level }, action, id),
        InteractReq::Obj {
            x,
            z,
            level,
            name,
            action,
            id,
        } => dispatch_obj(
            ix,
            context,
            WorldTile { x, z, level },
            name.as_deref(),
            action,
            id,
        ),
        InteractReq::Player { name, action } => {
            dispatch_player(ix, context.snapshot, &name, action)
        }
        InteractReq::UseOn {
            name,
            kind,
            target_name,
            x,
            z,
            level,
            index,
            source_item_id,
            source_item_slot,
            target_item_id,
            target_item_slot,
        } => dispatch_use_on(
            ix,
            context,
            UseOnArgs {
                name: &name,
                kind: &kind,
                target_name: target_name.as_deref(),
                x,
                z,
                level,
                index,
                source_item_id,
                source_item_slot,
                target_item_id,
                target_item_slot,
            },
        ),
        InteractReq::UseWidgetOn {
            component_id,
            kind,
            target_name,
            x,
            z,
            level,
            index,
            target_id,
        } => dispatch_use_widget_on(
            ix,
            context,
            component_id,
            OpTargetQuery {
                kind: &kind,
                target_name: target_name.as_deref(),
                target_item_id: None,
                target_item_slot: None,
                tile: WorldTile { x, z, level },
                index,
                target_id,
            },
        ),
        InteractReq::SayPublic { text } => {
            matches!(ix.say_public(&text), SendResult::Sent { .. })
        }
        InteractReq::ContinueDialog { component_id } => {
            matches!(ix.continue_dialog(component_id), SendResult::Sent { .. })
        }
        InteractReq::Answer { option } => {
            matches!(ix.answer_choice(option), SendResult::Sent { .. })
        }
        InteractReq::IfButton { component_id } => {
            dispatch_if_button(ix, context.snapshot, component_id)
        }
        InteractReq::DuelAccept {
            screen,
            partner,
            rules,
        } => dispatch_duel_accept(ix, context, revision, &screen, &partner, rules),
        InteractReq::CloseModal => matches!(ix.close_modal(), SendResult::Sent { .. }),
        InteractReq::AnswerCount { value } => {
            finish_bank_send(ix.answer_count(value), format_args!("count {value}"))
        }
        InteractReq::SideTab { tab } => matches!(ix.click_side_tab(tab), SendResult::Sent { .. }),
        InteractReq::Wear { name } => dispatch_wear(ix, context.snapshot, context.obj_names, &name),
        InteractReq::Unequip { name } => {
            dispatch_unequip(ix, context.snapshot, context.obj_names, &name)
        }
        InteractReq::SetRun { on } => matches!(ix.set_run(on), SendResult::Sent { .. }),
        InteractReq::SetRetaliate { on } => {
            matches!(ix.set_retaliate(on), SendResult::Sent { .. })
        }
        InteractReq::SetNoteMode { on } => {
            finish_bank_send(ix.set_note_mode(on), format_args!("note mode on={on}"))
        }
        InteractReq::Key { down, key, .. } => ix.apply_amount_key(down, &key),
        InteractReq::RunPolicyOverride { .. } => {
            // Applied only through Host's generation-fenced policy sink.
            false
        }
        InteractReq::GatherRun { .. }
        | InteractReq::GatherStop { .. }
        | InteractReq::CombatFight { .. }
        | InteractReq::CombatStop { .. }
        | InteractReq::CastNpc { .. }
        | InteractReq::ProgressRead { .. } => {
            // Host-local controls and typed native starts are consumed by the slot before game
            // dispatch; none can authorize a Driver packet here.
            false
        }
        InteractReq::Mouse { .. }
        | InteractReq::KeyGesture { .. }
        | InteractReq::SetCameraYaw { .. }
        | InteractReq::NoteProgress
        | InteractReq::LoopSettled
        | InteractReq::WaitEnqueued
        | InteractReq::WaitSettled
        | InteractReq::RecoveryAnchor { .. }
        | InteractReq::RecoveryAnchorNone
        | InteractReq::ChannelOpen { .. }
        | InteractReq::ChannelPost { .. }
        | InteractReq::ChannelClose { .. }
        | InteractReq::ChannelMessage { .. }
        | InteractReq::ChannelStatus { .. } => false,
    }
}

/// Parameters for one bank-item slot drag interaction.
struct BankDragArgs {
    id: i32,
    item_slot: i32,
    component: i32,
    target_slot: i32,
    insert: bool,
    bank_generation: u64,
}

/// Parameters for one shop buy/sell button click.
struct ShopButtonArgs<'a> {
    kind: &'a str,
    name: &'a str,
    id: i32,
    item_slot: i32,
    component: i32,
    chunk: i32,
}

/// Parameters for locating a scene, NPC, ground, or inventory op target.
struct OpTargetQuery<'a> {
    kind: &'a str,
    target_name: Option<&'a str>,
    target_item_id: Option<i32>,
    target_item_slot: Option<i32>,
    tile: WorldTile,
    index: Option<i32>,
    target_id: Option<i32>,
}

fn find_exact_item(
    items: &[api::snapshot::ItemView],
    id: i32,
    item_slot: i32,
    component: i32,
) -> Option<&api::snapshot::ItemView> {
    items
        .iter()
        .find(|item| item.def.id == id && item.slot == item_slot && item.component_id == component)
}

fn finish_bank_send(res: api::interact::SendResult, action: std::fmt::Arguments<'_>) -> bool {
    match &res {
        api::interact::SendResult::Sent { .. } => {
            host_log!(Category::BankOp, Level::Info, "{action}")
        }
        api::interact::SendResult::Refused { reason, .. } => {
            host_log!(Category::BankOp, Level::Warn, "{action} refused {reason:?}")
        }
    }
    matches!(res, api::interact::SendResult::Sent { .. })
}

fn dispatch_open_booth(
    ix: &mut api::interact::Interactions<'_>,
    context: &InteractContext<'_>,
    owner_slot: Option<&mut script::SlotScript>,
    target: WorldTile,
    id: i32,
    booth_name: Option<&str>,
    action: Option<&str>,
) -> bool {
    let accepted = match (booth_name, action) {
        (Some(name), Some(action)) => matches!(
            ix.open_named_booth_at(target, id, name, action),
            api::interact::SendResult::Sent { .. }
        ),
        (None, None) => matches!(
            ix.open_booth_at(target, id),
            api::interact::SendResult::Sent { .. }
        ),
        _ => false,
    };
    // The interaction supersedes any earlier scripted walk.
    // Cancel after validation/dispatch only: rejected or malformed requests
    // must not disturb an armed route.
    if accepted {
        retire_logical_walk(
            owner_slot,
            context.navs,
            context.name,
            None,
            script::native::WalkEnd::Cancelled,
        );
    }
    accepted
}

fn dispatch_walk_nearest_bank(context: &InteractContext<'_>) -> bool {
    let (Some((x, z, level)), Some(scope)) = (context.here, context.compatibility_scope) else {
        return false;
    };
    super::bank::queue_bank_walk_scoped(
        context.navs,
        context.name,
        scope,
        context.world,
        context.state,
        context.snapshot,
        WorldTile { x, z, level },
        context
            .snapshot
            .stats()
            .iter()
            .find(|stat| stat.index == 10)
            .map(|stat| stat.base),
    )
}

fn dispatch_walk_to(
    ix: &mut api::interact::Interactions<'_>,
    context: &InteractContext<'_>,
    dest: WorldTile,
) -> bool {
    let sent = matches!(
        ix.walk_nearest(dest),
        api::interact::SendResult::Sent { .. }
    );
    if sent {
        record_script_act(
            context.navs,
            context.name,
            ScriptAct::WalkTo {
                dest: act_tile(dest.x, dest.z, dest.level),
            },
        );
    }
    sent
}

fn dispatch_deposit(
    ix: &mut api::interact::Interactions<'_>,
    context: &InteractContext<'_>,
    name: &str,
    id: Option<i32>,
) -> bool {
    // The content's Deposit All operation moves every held copy of this item,
    // including copies in other inventory slots. A selected id takes only that
    // item type, never another row of the same name.
    let Some((item, op)) = context.snapshot.bank_side().iter().find_map(|item| {
        (id.is_none_or(|id| item.def.id == id)
            && context
                .obj_names
                .and_then(|names| names.name(item.def.id))
                .is_some_and(|actual| actual.eq_ignore_ascii_case(name)))
        .then(|| script::bank::ops::all_op_in(&item.actions).map(|op| (item, op)))
        .flatten()
    }) else {
        return false;
    };
    matches!(
        ix.interact(
            api::interact::OpTarget::Item(item),
            api::interact::ActionSpec::Operation(op),
        ),
        api::interact::SendResult::Sent { .. }
    )
}

fn dispatch_bank_drag(
    ix: &mut api::interact::Interactions<'_>,
    snapshot: &GameSnapshot,
    args: BankDragArgs,
) -> bool {
    // The exact posted bank row in the same bank session; the api re-checks
    // the grid and the target slot before sending.
    if !bank_session_matches(snapshot, args.bank_generation) {
        return false;
    }
    let Some(item) = find_exact_item(snapshot.bank(), args.id, args.item_slot, args.component)
    else {
        return false;
    };
    matches!(
        ix.drag_bank_item(item, args.target_slot, args.insert),
        api::interact::SendResult::Sent { .. }
    )
}

fn dispatch_shop_button(
    ix: &mut api::interact::Interactions<'_>,
    snapshot: &GameSnapshot,
    args: ShopButtonArgs<'_>,
) -> bool {
    // Sell acts on the shop's own player pack and Buy on its stock: never the
    // other container, never the backpack. The exact posted row must still be
    // there (no same-name fallback), then the api op re-resolves and sends it.
    let rows: &[api::snapshot::ItemView] = if args.kind == "sell" {
        &snapshot.shop().player
    } else {
        &snapshot.shop().stock
    };
    if find_exact_item(rows, args.id, args.item_slot, args.component).is_none() {
        return false;
    }
    let sent = if args.kind == "sell" {
        ix.shop_sell(args.name, args.chunk)
    } else {
        ix.shop_buy(args.name, args.chunk)
    };
    matches!(sent, api::interact::SendResult::Sent { .. })
}

fn dispatch_make_panel(
    ix: &mut api::interact::Interactions<'_>,
    snapshot: &GameSnapshot,
    id: i32,
    item_slot: i32,
    component: i32,
    operation: i32,
) -> bool {
    // Anvil/main skill-multi identity only. Do not fall back to
    // another same-name row, the backpack, or a count dialog.
    let Some(item) = find_exact_item(snapshot.main_make(), id, item_slot, component) else {
        return false;
    };
    matches!(
        ix.interact(
            api::interact::OpTarget::Item(item),
            api::interact::ActionSpec::Operation(operation),
        ),
        api::interact::SendResult::Sent { .. }
    )
}

fn dispatch_close_logged(ix: &mut api::interact::Interactions<'_>) -> bool {
    let res = ix.close_modal();
    if api::hostlog::enabled(Category::InteractTrace) {
        match &res {
            api::interact::SendResult::Sent { .. } => {
                host_log!(Category::InteractTrace, Level::Debug, "close sent")
            }
            api::interact::SendResult::Refused { reason, .. } => host_log!(
                Category::InteractTrace,
                Level::Debug,
                "close refused {reason:?}"
            ),
        }
    }
    matches!(res, api::interact::SendResult::Sent { .. })
}

fn dispatch_npc(
    ix: &mut api::interact::Interactions<'_>,
    context: &InteractContext<'_>,
    name: &str,
    action: String,
    index: Option<i32>,
) -> bool {
    let wanted = name.to_lowercase();
    let Some(npc) = context.snapshot.npcs().iter().find(|n| {
        if let Some(i) = index {
            n.index as i32 == i
        } else {
            n.name
                .as_deref()
                .is_some_and(|n| n.eq_ignore_ascii_case(&wanted))
        }
    }) else {
        return false;
    };
    let sent = matches!(
        ix.interact(
            api::interact::OpTarget::Npc(npc),
            api::interact::ActionSpec::Label(action.clone()),
        ),
        api::interact::SendResult::Sent { .. }
    );
    if sent {
        record_script_act(
            context.navs,
            context.name,
            ScriptAct::Npc {
                name: npc.name.clone().unwrap_or_default(),
                action,
                index: npc.index as i32,
            },
        );
    }
    sent
}

fn dispatch_loc(
    ix: &mut api::interact::Interactions<'_>,
    context: &InteractContext<'_>,
    tile: WorldTile,
    action: String,
    id: Option<i32>,
) -> bool {
    // Selected `id` must match a current row at this tile. Do not fall back
    // to another co-located loc. Absent id keeps the previous first-row
    // coordinate match.
    let Some(loc) = context.snapshot.locs().iter().find(|l| {
        l.tile.x == tile.x
            && l.tile.z == tile.z
            && l.tile.level == tile.level
            && id.is_none_or(|wanted| l.id == wanted)
    }) else {
        return false;
    };
    let sent = matches!(
        ix.interact(
            api::interact::OpTarget::Loc(loc),
            api::interact::ActionSpec::Label(action.clone()),
        ),
        api::interact::SendResult::Sent { .. }
    );
    if sent {
        record_script_act(
            context.navs,
            context.name,
            ScriptAct::Loc {
                tile: act_tile(loc.tile.x, loc.tile.z, loc.tile.level),
                id: loc.id,
                action,
            },
        );
    }
    sent
}

fn dispatch_obj(
    ix: &mut api::interact::Interactions<'_>,
    context: &InteractContext<'_>,
    tile: WorldTile,
    name: Option<&str>,
    action: String,
    id: Option<i32>,
) -> bool {
    let Some(obj) = context.snapshot.ground_items().iter().find(|it| {
        it.tile.x == tile.x
            && it.tile.z == tile.z
            && it.tile.level == tile.level
            && match id {
                Some(wanted) => it.def.id == wanted,
                None => name.is_none_or(|wanted| {
                    context
                        .obj_names
                        .and_then(|n| n.name(it.def.id))
                        .is_some_and(|n| n.eq_ignore_ascii_case(wanted))
                }),
            }
    }) else {
        return false;
    };
    matches!(
        ix.interact(
            api::interact::OpTarget::GroundItem(obj),
            api::interact::ActionSpec::Label(action),
        ),
        api::interact::SendResult::Sent { .. }
    )
}

fn dispatch_player(
    ix: &mut api::interact::Interactions<'_>,
    snapshot: &GameSnapshot,
    name: &str,
    action: String,
) -> bool {
    let wanted = name.to_lowercase();
    let Some(player) = snapshot.players().iter().find(|p| {
        p.actor
            .name
            .as_deref()
            .is_some_and(|n| n.eq_ignore_ascii_case(&wanted))
    }) else {
        return false;
    };
    matches!(
        ix.interact(
            api::interact::OpTarget::Player(player),
            api::interact::ActionSpec::Label(action),
        ),
        api::interact::SendResult::Sent { .. }
    )
}

fn dispatch_use_widget_on(
    ix: &mut api::interact::Interactions<'_>,
    context: &InteractContext<'_>,
    component_id: i32,
    query: OpTargetQuery<'_>,
) -> bool {
    let ctx = api::snapshot::ReadContext::new(context.snapshot);
    let Some(widget) = ctx.component(component_id) else {
        return false;
    };
    let Some(target) = resolve_op_target(context.snapshot, context.obj_names, query) else {
        return false;
    };
    matches!(
        ix.use_widget_on(widget, target),
        api::interact::SendResult::Sent { .. }
    )
}

fn dispatch_if_button(
    ix: &mut api::interact::Interactions<'_>,
    snapshot: &GameSnapshot,
    component_id: i32,
) -> bool {
    let ctx = api::snapshot::ReadContext::new(snapshot);
    if let Some(widget) = ctx.component(component_id) {
        let sent = matches!(ix.if_button(widget), api::interact::SendResult::Sent { .. });
        if host::debug_enabled() {
            eprintln!("[shim-if-button] {component_id} sent={sent}");
        }
        sent
    } else {
        if host::debug_enabled() {
            eprintln!("[shim-if-button] {component_id} missing");
        }
        false
    }
}

fn bank_session_matches(snapshot: &GameSnapshot, bank_generation: u64) -> bool {
    snapshot.bank_component_id() >= 0
        && snapshot.bank_loaded()
        && snapshot.bank_session_generation() == bank_generation
}

fn dispatch_open_stand(
    ix: &mut api::interact::Interactions<'_>,
    context: &InteractContext<'_>,
    owner_slot: Option<&mut script::SlotScript>,
    tile: WorldTile,
    kind: &str,
    name: Option<&str>,
    stand_op: Option<i32>,
) -> bool {
    use api::interact::{ActionSpec, OpTarget, SendResult};
    let accepted = if kind == "booth" {
        let loc = script::bank::ops::booth_at(context.snapshot.locs(), tile, name);
        loc.is_some_and(|loc| {
            let op = stand_op.or_else(|| {
                script::bank::ops::op_slot(
                    loc,
                    "Use-quickly",
                    script::bank::ops::LabelMatch::Normalized,
                )
            });
            op.is_some_and(|op| {
                matches!(
                    ix.interact(OpTarget::Loc(loc), ActionSpec::Operation(op)),
                    SendResult::Sent { .. }
                )
            })
        })
    } else if kind == "npc" {
        let npc = name
            .and_then(|wanted| script::bank::ops::npc_at(context.snapshot.npcs(), tile, wanted));
        npc.is_some_and(|npc| {
            let op = stand_op
                .and_then(|op| script::bank::ops::packed_op(npc, op))
                .or_else(|| {
                    script::bank::ops::op_slot(
                        npc,
                        "Bank",
                        script::bank::ops::LabelMatch::Normalized,
                    )
                });
            op.is_some_and(|op| {
                matches!(
                    ix.interact(OpTarget::Npc(npc), ActionSpec::Operation(op)),
                    SendResult::Sent { .. }
                )
            })
        })
    } else {
        false
    };
    // The interaction supersedes any earlier scripted walk.
    // Cancel after validation/dispatch only: rejected or malformed requests
    // must not disturb an armed route.
    if accepted {
        retire_logical_walk(
            owner_slot,
            context.navs,
            context.name,
            None,
            script::native::WalkEnd::Cancelled,
        );
    }
    accepted
}

/// Parameters for arming one scripted navigation walk request.
struct WalkRequestArgs {
    dest: WorldTile,
    radius: i32,
    exact: bool,
    compat_tile_intent: bool,
    allow_teleports: bool,
    allow_wilderness: bool,
    allow_bank_fetch: bool,
    allow_prereq_dialogue: Option<bool>,
    allow_quest_start: Option<bool>,
    request_id: u64,
    track_door_crossing: bool,
    avoid: Vec<script::interact::InspectAvoidWire>,
    cross: Vec<String>,
}

fn dispatch_walk_request(
    context: &InteractContext<'_>,
    owner_slot: Option<&mut script::SlotScript>,
    args: WalkRequestArgs,
) -> bool {
    let InteractContext {
        snapshot,
        here,
        navs,
        world,
        state,
        bank,
        name,
        ..
    } = *context;
    let (opts, danger) = super::walk_permissions::compiled_options(
        navs,
        name,
        FindOptions {
            allow_teleports: args.allow_teleports,
            allow_wilderness: args.allow_wilderness,
            allow_bank_fetch: args.allow_bank_fetch,
            ..FindOptions::default()
        },
        !args.cross.is_empty(),
    );
    let allow_teleports = opts.allow_teleports;
    let allow_wilderness = opts.allow_wilderness;
    let allow_bank_fetch = opts.allow_bank_fetch;
    let key = (
        args.dest,
        args.radius,
        allow_teleports,
        allow_wilderness,
        allow_bank_fetch,
    );
    let exclusions = route_exclusions(args.avoid, args.cross);
    if already_following(navs, name, args.request_id, key, &exclusions) {
        return false;
    }
    if navs
        .lock()
        .unwrap()
        .get(name)
        .is_some_and(|bot| bot.compat_walk_request_id().is_some())
    {
        retire_logical_walk(
            owner_slot,
            navs,
            name,
            None,
            script::native::WalkEnd::Cancelled,
        );
    }
    let prereq_allow = (args.request_id != 0).then(|| {
        super::walk_permissions::compiled_prereq_allow(
            navs,
            name,
            args.allow_prereq_dialogue,
            args.allow_quest_start,
        )
    });
    let arm = ScriptWalkArm {
        here,
        world: world.clone(),
        navs: Arc::clone(navs),
        name: name.to_string(),
        state: state.cloned(),
        bank: super::slot_bank_memory::planner_rows(bank),
    };
    let queued = if args.exact {
        arm.queue_fixed_route_in_snapshot_avoiding(
            snapshot,
            args.dest.x,
            args.dest.z,
            args.dest.level,
            opts,
            args.request_id,
            exclusions,
            danger,
        )
    } else {
        arm.queue_route_in_snapshot_avoiding(
            snapshot,
            args.dest.x,
            args.dest.z,
            args.dest.level,
            opts,
            args.radius,
            args.request_id,
            args.compat_tile_intent,
            exclusions,
            danger,
        )
    };
    if queued {
        if let Some(bot) = navs.lock().unwrap().get_mut(name) {
            bot.arm_door_crossing(args.request_id, args.track_door_crossing);
            if let Some(prereq_allow) = prereq_allow {
                bot.retain_compat_walk(
                    args.request_id,
                    script::native::WalkOptions {
                        allow_prereq_dialogue: args.allow_prereq_dialogue.into(),
                        allow_quest_start: args.allow_quest_start.into(),
                        ..Default::default()
                    },
                    prereq_allow,
                );
            }
        }
        record_script_act(
            navs,
            name,
            ScriptAct::Walk {
                dest: act_tile(args.dest.x, args.dest.z, args.dest.level),
                radius: args.radius,
                exact: args.exact,
                allow_teleports,
                allow_wilderness,
                allow_bank_fetch,
                request_id: args.request_id,
            },
        );
    }
    queued
}

/// Parameters for queueing one route-inspection query.
struct InspectRouteArgs {
    from: WorldTile,
    to: WorldTile,
    allow_teleports: bool,
    allow_wilderness: bool,
    allow_bank_fetch: bool,
    avoid: Vec<script::interact::InspectAvoidWire>,
    cross: Vec<String>,
    request_id: u64,
    max_expansions: Option<u64>,
}

fn dispatch_inspect_route(context: &InteractContext<'_>, args: InspectRouteArgs) {
    let InteractContext {
        snapshot,
        navs,
        world,
        state,
        bank,
        name,
        cache,
        obj_names_arc,
        ..
    } = *context;
    let quest_context = navs
        .lock()
        .unwrap()
        .get(name)
        .and_then(|bot| bot.quest_context.clone());
    let route_state = super::script_nav::route_world_state(snapshot, state.cloned());
    let route_state = match super::script_nav::patch_route_quest_evidence(
        route_state,
        super::script_nav::captured_service_quests(quest_context.as_ref(), snapshot, None),
    ) {
        Ok(state) => Some(state),
        Err(_) => {
            host_log!(
                Category::NavTrace,
                Level::Warn,
                slot = name,
                "inspect route refused quest evidence allocation"
            );
            None
        }
    };
    let state_capture_failed = route_state.is_none();
    let (opts, _) = super::walk_permissions::compiled_options(
        navs,
        name,
        FindOptions {
            allow_teleports: args.allow_teleports,
            allow_wilderness: args.allow_wilderness,
            allow_bank_fetch: args.allow_bank_fetch,
            ..FindOptions::default()
        },
        !args.cross.is_empty(),
    );
    let (zones, rects, invalid_args) = match (world.as_ref(), route_state.as_deref()) {
        (Some(world), Some(route_state)) => match super::script_nav::resolve_route_exclusions(
            world,
            args.from,
            args.to,
            route_state,
            route_exclusions(args.avoid, args.cross),
        ) {
            Ok(exclusions) => (opts.zones, exclusions.avoid, false),
            Err(reason) => {
                host_log!(
                    Category::NavTrace,
                    Level::Warn,
                    slot = name,
                    "inspect route refused {reason}"
                );
                (nav_planner::zones::ZoneExempt::NONE, Vec::new(), true)
            }
        },
        (Some(_), None) => (nav_planner::zones::ZoneExempt::NONE, Vec::new(), false),
        (None, _) => (nav_planner::zones::ZoneExempt::NONE, Vec::new(), true),
    };
    route_inspect::queue_inspect_shared(
        navs,
        name,
        world,
        route_state.as_ref(),
        super::slot_bank_memory::planner_rows(bank),
        cache.clone(),
        obj_names_arc.clone(),
        route_inspect::InspectRequest {
            from: args.from,
            to: args.to,
            allow_teleports: opts.allow_teleports,
            allow_wilderness: opts.allow_wilderness,
            allow_bank_fetch: opts.allow_bank_fetch,
            zones,
            avoid: rects,
            request_id: args.request_id,
            max_expansions: args.max_expansions,
            invalid_args,
            state_capture_failed,
        },
    );
}

fn dispatch_withdraw(
    ix: &mut api::interact::Interactions<'_>,
    snapshot: &GameSnapshot,
    obj_names: Option<&api::obj_names::ObjNames>,
    name: &str,
    action: &str,
) -> bool {
    use api::interact::{ActionSpec, OpTarget, SendResult};
    let wanted = name.to_lowercase();
    let Some(item) = snapshot.bank().iter().find(|it| {
        obj_names
            .and_then(|n| n.name(it.def.id))
            .is_some_and(|n| n.eq_ignore_ascii_case(&wanted))
    }) else {
        host_log!(
            Category::BankOp,
            Level::Warn,
            "withdraw {name} {action}: not in bank"
        );
        return false;
    };
    let Some(op) = script::bank::ops::action_op_in(&item.actions, action) else {
        host_log!(
            Category::BankOp,
            Level::Warn,
            "withdraw {name} {action}: no such action"
        );
        return false;
    };
    let res = ix.interact(OpTarget::Item(item), ActionSpec::Operation(op));
    match &res {
        SendResult::Sent { .. } => {
            host_log!(Category::BankOp, Level::Info, "withdraw {name} {action}")
        }
        SendResult::Refused { reason, .. } => host_log!(
            Category::BankOp,
            Level::Warn,
            "withdraw {name} {action} refused {reason:?}"
        ),
    }
    matches!(res, SendResult::Sent { .. })
}

fn dispatch_held(
    ix: &mut api::interact::Interactions<'_>,
    snapshot: &GameSnapshot,
    obj_names: Option<&api::obj_names::ObjNames>,
    name: &str,
    action: &str,
    slot: Option<i32>,
    target_item_id: Option<i32>,
) -> bool {
    use api::interact::{ActionSpec, OpTarget, SendResult};
    // An explicit id requires the exact id/slot pair. Requests without one
    // retain the legacy name rule, constrained by an optional native-only slot.
    let item = if target_item_id.is_some() {
        resolve_inventory_item(snapshot, obj_names, Some(name), target_item_id, slot)
    } else {
        let wanted = name.to_lowercase();
        snapshot.inventory().iter().find(|it| {
            it.count > 0
                && slot.is_none_or(|slot| it.slot == slot)
                && obj_names
                    .and_then(|n| n.name(it.def.id))
                    .is_some_and(|n| n.eq_ignore_ascii_case(&wanted))
        })
    };
    let Some(item) = item.filter(|item| item.count > 0) else {
        return false;
    };
    let res = ix.interact(OpTarget::Item(item), ActionSpec::Label(action.to_owned()));
    if api::hostlog::enabled(Category::InteractTrace) {
        let outcome = match &res {
            SendResult::Sent { .. } => "sent".to_string(),
            SendResult::Refused { reason, .. } => format!("refused {reason:?}"),
        };
        host_log!(
            Category::InteractTrace,
            Level::Debug,
            "held {name} {action} slot={} -> {outcome}",
            item.slot
        );
    }
    matches!(res, SendResult::Sent { .. })
}

fn dispatch_inv_button(
    ix: &mut api::interact::Interactions<'_>,
    snapshot: &GameSnapshot,
    id: i32,
    slot: i32,
    component: i32,
    operation: i32,
    bank_generation: u64,
) -> bool {
    use api::interact::{ActionSpec, OpTarget, SendResult};
    // Selected bank withdraw or bank-side deposit identity, or the open trade
    // offer/side row. Exact id/slot/component only — no same-name fallback, no
    // count-dialog answer.
    let matches_row = |item: &&api::snapshot::ItemView| {
        item.def.id == id && item.slot == slot && item.component_id == component
    };
    let item = if bank_session_matches(snapshot, bank_generation) {
        snapshot
            .bank()
            .iter()
            .chain(snapshot.bank_side().iter())
            .find(matches_row)
    } else if snapshot.trade().offer_open {
        snapshot
            .trade()
            .side_pack
            .iter()
            .chain(snapshot.trade().my_offer.iter())
            .find(matches_row)
    } else {
        None
    };
    item.is_some_and(|item| {
        matches!(
            ix.interact(OpTarget::Item(item), ActionSpec::Operation(operation)),
            SendResult::Sent { .. }
        )
    })
}

/// Parameters for dispatching one `UseOn` item-on-target interaction.
struct UseOnArgs<'a> {
    name: &'a str,
    kind: &'a str,
    target_name: Option<&'a str>,
    x: i32,
    z: i32,
    level: i32,
    index: Option<i32>,
    source_item_id: Option<i32>,
    source_item_slot: Option<i32>,
    target_item_id: Option<i32>,
    target_item_slot: Option<i32>,
}

fn dispatch_use_on(
    ix: &mut api::interact::Interactions<'_>,
    context: &InteractContext<'_>,
    args: UseOnArgs<'_>,
) -> bool {
    use api::interact::{OpTarget, SendResult};
    let Some(item) = resolve_inventory_item(
        context.snapshot,
        context.obj_names,
        Some(args.name),
        args.source_item_id,
        args.source_item_slot,
    ) else {
        return false;
    };
    let Some(target) = resolve_op_target(
        context.snapshot,
        context.obj_names,
        OpTargetQuery {
            kind: args.kind,
            target_name: args.target_name,
            target_item_id: args.target_item_id,
            target_item_slot: args.target_item_slot,
            tile: WorldTile {
                x: args.x,
                z: args.z,
                level: args.level,
            },
            index: args.index,
            target_id: None,
        },
    ) else {
        return false;
    };
    let on_loc = match &target {
        OpTarget::Loc(loc) => Some(act_tile(loc.tile.x, loc.tile.z, loc.tile.level)),
        _ => None,
    };
    let item_name = context
        .obj_names
        .and_then(|names| names.name(item.def.id))
        .map_or_else(|| args.name.to_owned(), str::to_string);
    let sent = matches!(ix.use_item_on(item, target), SendResult::Sent { .. });
    if let (true, Some(tile)) = (sent, on_loc) {
        record_script_act(
            context.navs,
            context.name,
            ScriptAct::UseOnLoc {
                item: item_name,
                tile,
            },
        );
    }
    sent
}

fn dispatch_duel_accept(
    ix: &mut api::interact::Interactions<'_>,
    context: &InteractContext<'_>,
    revision: api::selected::ClientRevision,
    screen: &str,
    partner: &str,
    rules: i32,
) -> bool {
    use api::interact::SendResult;
    let controls = api::game_data::for_revision(revision)
        .ok()
        .and_then(|data| data.duel_controls().copied());
    let accept = controls.and_then(|controls| {
        let mut navs = context.navs.lock().unwrap();
        let offer = &mut navs
            .entry(context.name.to_string())
            .or_default()
            .duel_offer_partner;
        validated_duel_accept(context.snapshot, &controls, offer, screen, partner, rules)
    });
    let Some(component_id) = accept else {
        if host::debug_enabled() {
            eprintln!(
                "[shim-duel-accept] refused screen={screen:?} partner={partner:?} rules={rules}"
            );
        }
        return false;
    };
    let ctx = api::snapshot::ReadContext::new(context.snapshot);
    let Some(widget) = ctx.component(component_id) else {
        if host::debug_enabled() {
            eprintln!("[shim-duel-accept] screen={screen:?} component={component_id} missing");
        }
        return false;
    };
    let sent = matches!(ix.if_button(widget), SendResult::Sent { .. });
    if host::debug_enabled() {
        eprintln!("[shim-duel-accept] screen={screen:?} component={component_id} sent={sent}");
    }
    if sent && screen == "confirm" {
        if let Some(bot) = context.navs.lock().unwrap().get_mut(context.name) {
            bot.duel_offer_partner = None;
        }
    }
    sent
}

fn dispatch_wear(
    ix: &mut api::interact::Interactions<'_>,
    snapshot: &GameSnapshot,
    obj_names: Option<&api::obj_names::ObjNames>,
    name: &str,
) -> bool {
    use api::interact::SendResult;
    let wanted = name.to_lowercase();
    let Some(id) = snapshot.inventory().iter().find_map(|it| {
        obj_names
            .and_then(|n| n.name(it.def.id))
            .filter(|n| n.eq_ignore_ascii_case(&wanted))
            .map(|_| it.def.id)
    }) else {
        host_log!(
            Category::InteractTrace,
            Level::Debug,
            "wear {name}: no inventory() row inv={} zip={:?}",
            snapshot.inventory().len(),
            snapshot.inv()
        );
        return false;
    };
    let res = ix.wear(id);
    if api::hostlog::enabled(Category::InteractTrace) {
        let outcome = match &res {
            SendResult::Sent { .. } => "sent".to_string(),
            SendResult::Refused { reason, .. } => format!("refused {reason:?}"),
        };
        host_log!(
            Category::InteractTrace,
            Level::Debug,
            "wear {name} id={id} inv={} zip={} -> {outcome}",
            snapshot.inventory().len(),
            snapshot.inv().len()
        );
    }
    matches!(res, SendResult::Sent { .. })
}

fn dispatch_unequip(
    ix: &mut api::interact::Interactions<'_>,
    snapshot: &GameSnapshot,
    obj_names: Option<&api::obj_names::ObjNames>,
    name: &str,
) -> bool {
    use api::interact::SendResult;
    let Some(id) = snapshot.equipment().iter().find_map(|it| {
        obj_names
            .and_then(|n| n.name(it.def.id))
            .filter(|n| n.eq_ignore_ascii_case(name))
            .map(|_| it.def.id)
    }) else {
        host_log!(
            Category::InteractTrace,
            Level::Debug,
            "unequip {name}: no equipment() row worn={}",
            snapshot.equipment().len()
        );
        return false;
    };
    let res = ix.unequip(id);
    if api::hostlog::enabled(Category::InteractTrace) {
        let outcome = match &res {
            SendResult::Sent { .. } => "sent".to_string(),
            SendResult::Refused { reason, .. } => format!("refused {reason:?}"),
        };
        host_log!(
            Category::InteractTrace,
            Level::Debug,
            "unequip {name} id={id} worn={} -> {outcome}",
            snapshot.equipment().len()
        );
    }
    matches!(res, SendResult::Sent { .. })
}

/// The frozen `PUZZLE_SIZE` (5×5) a live puzzle board reports. The posted
/// size stays the observation; this is only the puzzle-move send gate.
const PUZZLE_SIZE: i32 = 25;

/// One `puzzle-move` click: the exact posted board row (id/slot/component)
/// under its own `puzzle_session_generation`, sent as a send-time
/// Held-family view (`Move`, else the frozen index-5 op) through the
/// OPHELD opcode family. A closed board, a board whose observed size is not
/// the frozen 25, a stale generation, a row the board no longer holds, a
/// missing obj table and a fifth op that is only the padded `Drop` default
/// all send nothing — and a sent packet is not board progress, so nothing
/// here waits on or rewrites the board.
fn dispatch_puzzle_move(
    ix: &mut api::interact::Interactions<'_>,
    snapshot: &GameSnapshot,
    cache: Option<&Cache>,
    id: i32,
    slot: i32,
    component: i32,
    generation: u64,
) -> bool {
    use api::interact::{ActionSpec, OpTarget, SendResult};
    // The posted board's own row under its own session generation: never a
    // name, never a co-located widget, and never the bank session. A refused
    // or stale move sends nothing and closes nothing.
    let board = snapshot.puzzle_board();
    if component < 0 || board.component_id != component {
        return false;
    }
    if board.generation != generation || board.size != PUZZLE_SIZE {
        return false;
    }
    let Some(row) = board
        .items
        .iter()
        .find(|item| item.def.id == id && item.slot == slot && item.component_id == component)
    else {
        return false;
    };
    let Some(cache) = cache else {
        return false;
    };
    let view = api::snapshot::ItemView {
        def: row.def.clone(),
        container: api::snapshot::ItemContainer::Widget,
        action_family: api::snapshot::ItemActionFamily::Held,
        slot: row.slot,
        count: row.count,
        actions: api::snapshot::cache_held_ops(cache, id),
        component_id: row.component_id,
    };
    let target = OpTarget::Item(&view);
    let action = if api::interact::operation_of(&target, "Move").is_some() {
        // The frozen click's labelled path: `Move` wins wherever the obj
        // table has it, and `interact` re-resolves the same slot.
        ActionSpec::Label("Move".into())
    } else {
        // Every piece defines `iop5=Move` (the frozen click's no-label
        // path), but `cache_held_ops` pads a `Drop` into the fifth slot
        // when the obj table has none and `offers_operation(5)` treats that
        // default as usable. Only a def-defined, non-drop fifth op is the
        // index-5 fallback; the padded default is never sent as OPHELD5.
        let fifth = cache
            .objs
            .get(id as usize)
            .and_then(|obj| obj.iop.get(4))
            .and_then(|op| op.as_deref());
        if !fifth.is_some_and(|label| !label.trim().eq_ignore_ascii_case("Drop")) {
            return false;
        }
        ActionSpec::Operation(5)
    };
    matches!(ix.interact(target, action), SendResult::Sent { .. })
}

fn resolve_op_target<'a>(
    snapshot: &'a GameSnapshot,
    obj_names: Option<&api::obj_names::ObjNames>,
    query: OpTargetQuery<'_>,
) -> Option<api::interact::OpTarget<'a>> {
    use api::interact::OpTarget;
    let OpTargetQuery {
        kind,
        target_name,
        target_item_id,
        target_item_slot,
        tile,
        index,
        target_id,
    } = query;
    match kind {
        "npc" if target_id.is_some() => {
            let wanted_type = usize::try_from(target_id?).ok()?;
            let wanted_index = usize::try_from(index?).ok()?;
            snapshot
                .npcs()
                .iter()
                .find(|npc| {
                    npc.index == wanted_index
                        && npc.r#type == Some(wanted_type)
                        && npc.network.x == tile.x
                        && npc.network.z == tile.z
                        && npc.network.level == tile.level
                        && target_name.is_none_or(|wanted| {
                            npc.name
                                .as_deref()
                                .is_some_and(|name| name.eq_ignore_ascii_case(wanted))
                        })
                })
                .map(OpTarget::Npc)
        }
        "npc" => {
            let wanted = target_name.map(str::to_lowercase);
            snapshot
                .npcs()
                .iter()
                .find(|n| {
                    if let Some(i) = index {
                        n.index as i32 == i
                    } else {
                        wanted.as_ref().is_some_and(|w| {
                            n.name.as_deref().is_some_and(|n| n.eq_ignore_ascii_case(w))
                        })
                    }
                })
                .map(OpTarget::Npc)
        }
        "loc" => snapshot
            .locs()
            .iter()
            .find(|loc| {
                loc.tile.x == tile.x
                    && loc.tile.z == tile.z
                    && loc.tile.level == tile.level
                    && target_item_id.is_none_or(|id| loc.id == id)
                    && target_name.is_none_or(|wanted| {
                        loc.name
                            .as_deref()
                            .is_some_and(|name| name.eq_ignore_ascii_case(wanted))
                    })
            })
            .map(OpTarget::Loc),
        "obj" => snapshot
            .ground_items()
            .iter()
            .find(|it| {
                it.tile.x == tile.x
                    && it.tile.z == tile.z
                    && it.tile.level == tile.level
                    && target_item_id.is_none_or(|id| it.def.id == id)
                    && target_name.is_none_or(|wanted| {
                        obj_names
                            .and_then(|n| n.name(it.def.id))
                            .is_some_and(|n| n.eq_ignore_ascii_case(wanted))
                    })
            })
            .map(OpTarget::GroundItem),
        "player" => {
            let wanted = target_name.map(|n| n.to_lowercase());
            snapshot
                .players()
                .iter()
                .find(|p| {
                    wanted.as_ref().is_some_and(|w| {
                        p.actor
                            .name
                            .as_deref()
                            .is_some_and(|n| n.eq_ignore_ascii_case(w))
                    })
                })
                .map(OpTarget::Player)
        }
        "inv" | "held" => resolve_inventory_item(
            snapshot,
            obj_names,
            target_name,
            target_item_id,
            target_item_slot,
        )
        .map(OpTarget::Item),
        _ => None,
    }
}

fn resolve_inventory_item<'a>(
    snapshot: &'a GameSnapshot,
    obj_names: Option<&api::obj_names::ObjNames>,
    name: Option<&str>,
    item_id: Option<i32>,
    item_slot: Option<i32>,
) -> Option<&'a api::snapshot::ItemView> {
    match (item_id, item_slot) {
        (Some(id), Some(slot)) => snapshot
            .inventory()
            .iter()
            .find(|item| item.def.id == id && item.slot == slot),
        (None, None) => {
            let wanted = name?.to_lowercase();
            snapshot.inventory().iter().find(|item| {
                obj_names
                    .and_then(|names| names.name(item.def.id))
                    .is_some_and(|actual| actual.eq_ignore_ascii_case(&wanted))
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod ground_use_target_tests {
    use super::*;
    use api::interact::OpTarget;

    #[test]
    fn ground_use_target_keeps_exact_id_when_items_share_a_tile_and_name() {
        let tile = api::WorldTile {
            x: 2613,
            z: 9639,
            level: 0,
        };
        let ground = |id| api::snapshot::GroundItemView {
            def: api::obj_names::ItemDefView {
                id,
                name: Some("Cog".into()),
                stackable: false,
                members: true,
                base_value: 1,
                noted: false,
                certificate_link: -1,
                certificate_template: -1,
            },
            count: 1,
            actions: vec![Some("Take".into())],
            tile,
            distance: 1,
        };
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ground_items(vec![ground(10), ground(11)]);
        let target = resolve_op_target(
            &snapshot,
            None,
            OpTargetQuery {
                kind: "obj",
                target_name: None,
                target_item_id: Some(11),
                target_item_slot: None,
                tile,
                index: None,
                target_id: None,
            },
        );
        assert!(matches!(target, Some(OpTarget::GroundItem(item)) if item.def.id == 11));
        assert!(
            resolve_op_target(
                &snapshot,
                None,
                OpTargetQuery {
                    kind: "obj",
                    target_name: None,
                    target_item_id: Some(12),
                    target_item_slot: None,
                    tile,
                    index: None,
                    target_id: None,
                },
            )
            .is_none(),
            "a missing exact id must not select the same-name decoy"
        );
    }
}
