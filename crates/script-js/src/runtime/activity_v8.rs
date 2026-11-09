//! Native ScriptActivity callbacks over the slot-owned run context.

use crate::materialize::callback_v8::{self as cb, JsResult, Throw};
use crate::materialize::lineage_v8::{self, ActivityId};
use rustyscript::{deno_core::v8, Runtime};
use script::surface::ScriptRuntimeContext;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

const DISPATCHER: &str = "__rs2b0t_script_activity";

struct ActiveActivity {
    generation: u64,
    child_id: u64,
    owner: v8::Global<v8::Object>,
    child: v8::Global<v8::Value>,
    log: v8::Global<v8::Value>,
    meta: v8::Global<v8::Value>,
}
struct PendingOuter {
    child_id: u64,
    resolver: v8::Global<v8::PromiseResolver>,
}

struct ActivityState {
    context: RefCell<Option<Arc<ScriptRuntimeContext>>>,
    next_child_id: Cell<u64>,
    active: RefCell<Option<ActiveActivity>>,
    pending_outer: RefCell<Option<PendingOuter>>,
    last_owner: RefCell<Option<v8::Global<v8::Object>>>,
    randoms_override: Cell<bool>,
}

impl ActivityState {
    fn new() -> Self {
        Self {
            context: RefCell::new(None),
            next_child_id: Cell::new(1),
            active: RefCell::new(None),
            pending_outer: RefCell::new(None),
            last_owner: RefCell::new(None),
            randoms_override: Cell::new(false),
        }
    }
}

/// Install the isolate-local activity dispatcher.
pub(crate) fn install(runtime: &mut Runtime) -> Result<(), String> {
    let state = Rc::new(ActivityState::new());
    if !runtime.deno_runtime().v8_isolate().set_slot(state) {
        return Err("activity state already installed".into());
    }
    cb::install(runtime, DISPATCHER, activity_callback)
}

/// Bind the latest slot-owned generation before a script can use the handle.
pub(crate) fn bind_runtime_context(
    runtime: &mut Runtime,
    context: Arc<ScriptRuntimeContext>,
) -> Result<(), String> {
    let scope = runtime.deno_runtime().handle_scope();
    let state = scope
        .get_slot::<Rc<ActivityState>>()
        .cloned()
        .ok_or_else(|| "activity state is not installed".to_string())?;
    state.active.borrow_mut().take();
    state.last_owner.borrow_mut().take();
    state.randoms_override.set(false);
    *state.context.borrow_mut() = Some(context);
    Ok(())
}

/// Release V8 handles before the isolate is retired.
pub(crate) fn clear_all(runtime: &mut Runtime) {
    let scope = runtime.deno_runtime().handle_scope();
    if let Some(state) = scope.get_slot::<Rc<ActivityState>>().cloned() {
        state.active.borrow_mut().take();
        state.last_owner.borrow_mut().take();
        state.pending_outer.borrow_mut().take();
        state.context.borrow_mut().take();
        state.randoms_override.set(false);
    }
}

fn state_for_scope(scope: &v8::HandleScope) -> Option<Rc<ActivityState>> {
    scope.get_slot::<Rc<ActivityState>>().cloned()
}

fn context_for_generation(
    state: &ActivityState,
    generation: u64,
) -> Option<Arc<ScriptRuntimeContext>> {
    let context = state.context.borrow().as_ref().map(Arc::clone)?;
    (context.snapshot().generation == generation).then_some(context)
}

fn generation_arg(value: v8::Local<v8::Value>) -> Option<u64> {
    let token = v8::Local::<v8::BigInt>::try_from(value).ok()?;
    let (generation, lossless) = token.u64_value();
    lossless.then_some(generation)
}

fn owner_arg<'s>(
    scope: &mut v8::HandleScope<'s>,
    value: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Object>> {
    if value.is_null_or_undefined() {
        return Err(cb::type_error(
            scope,
            "ScriptActivity receiver is not an object",
        ));
    }
    v8::Local::<v8::Object>::try_from(value)
        .map_err(|_| cb::type_error(scope, "ScriptActivity receiver is not an object"))
}
fn instance_of<'s>(
    scope: &mut v8::HandleScope<'s>,
    value: v8::Local<'s, v8::Value>,
    constructor: v8::Local<'s, v8::Value>,
) -> JsResult<'s, bool> {
    let constructor = v8::Local::<v8::Function>::try_from(constructor)
        .map_err(|_| cb::type_error(scope, "ScriptActivity error constructor is unavailable"))?;
    cb::catching(scope, |scope| value.instance_of(scope, constructor.into()))
}

fn activity_callback<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let operation = args.get(0);
    let operation = if operation.is_string() {
        operation.to_rust_string_lossy(scope)
    } else {
        String::new()
    };
    let result = match operation.as_str() {
        "start" => start_promise(scope, args).map(Into::into),
        "loop" => loop_promise(scope, args).map(Into::into),
        _ => dispatch(scope, args),
    };
    cb::finish(scope, rv, result);
}

fn dispatch<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let operation = args.get(0);
    if !operation.is_string() {
        return Err(cb::type_error(
            scope,
            "ScriptActivity operation must be a string",
        ));
    }
    let operation = operation.to_rust_string_lossy(scope);
    let state = state_for_scope(scope)
        .ok_or_else(|| cb::type_error(scope, "ScriptActivity state is not installed"))?;
    match operation.as_str() {
        "construct" => {
            let context = state.context.borrow().as_ref().map(Arc::clone);
            match context {
                Some(context) => {
                    Ok(v8::BigInt::new_from_u64(scope, context.snapshot().generation).into())
                }
                None => Ok(v8::null(scope).into()),
            }
        }
        "child" => child(scope, &state, args),
        "outcome" => outcome(scope, &state, args),
        "currentTask" => current_task(scope, &state, args),
        "stop" => stop(scope, &state, args),
        "interrupt" => interrupt(scope, &state, args),
        "ignoredRandoms" => ignored_randoms(scope, &state),
        _ => Err(cb::type_error(scope, "unknown ScriptActivity operation")),
    }
}

fn new_promise<'s>(
    scope: &mut v8::HandleScope<'s>,
) -> JsResult<
    's,
    (
        v8::Local<'s, v8::PromiseResolver>,
        v8::Local<'s, v8::Promise>,
    ),
> {
    let resolver = v8::PromiseResolver::new(scope)
        .ok_or_else(|| cb::type_error(scope, "could not create ScriptActivity promise"))?;
    let promise = resolver.get_promise(scope);
    Ok((resolver, promise))
}

fn settle_resolver<'s>(
    scope: &mut v8::HandleScope<'s>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
    value: v8::Local<'s, v8::Value>,
    reject: bool,
) -> JsResult<'s, ()> {
    if reject {
        cb::catching(scope, |scope| resolver.reject(scope, value))?;
    } else {
        cb::catching(scope, |scope| resolver.resolve(scope, value))?;
    }
    Ok(())
}

fn error_value<'s>(
    scope: &mut v8::HandleScope<'s>,
    message: &str,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let global = scope.get_current_context().global(scope);
    let constructor = cb::get(scope, global.into(), "Error")?;
    let message = cb::string(scope, message);
    cb::construct(scope, constructor, &[message], "Error")
}

fn start_promise<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
) -> JsResult<'s, v8::Local<'s, v8::Promise>> {
    let (resolver, promise) = new_promise(scope)?;
    match start_inner(scope, args, resolver) {
        Ok(()) => Ok(promise),
        Err(Throw::Value(error)) => {
            settle_resolver(scope, resolver, error, true)?;
            Ok(promise)
        }
        Err(Throw::Terminated) => Err(Throw::Terminated),
    }
}

fn start_inner<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    outer_resolver: v8::Local<'s, v8::PromiseResolver>,
) -> JsResult<'s, ()> {
    let state = state_for_scope(scope)
        .ok_or_else(|| cb::type_error(scope, "ScriptActivity state is not installed"))?;
    let owner = owner_arg(scope, args.get(1))?;
    let Some(generation) = generation_arg(args.get(2)) else {
        return Err(Throw::Value(error_value(scope, "Script run has ended")?));
    };
    let Some(context) = context_for_generation(&state, generation) else {
        return Err(Throw::Value(error_value(scope, "Script run has ended")?));
    };
    let current = context.snapshot().activity;
    if current.child_id.is_some() || current.in_flight || state.active.borrow().is_some() {
        return Err(Throw::Value(error_value(
            scope,
            "An activity is already running",
        )?));
    }

    let meta = args.get(3);
    let overrides = args.get(4);
    let log = args.get(5);
    let looping_bot = args.get(6);
    let settings_constructor = args.get(7);
    let sustain = args.get(8);
    let event_signal = args.get(9);
    let run_manager = args.get(10);
    let scheduler = args.get(11);
    let script_aborted = args.get(12);
    let bot_host = args.get(13);
    let activity_finished = args.get(14);

    let create = cb::get(scope, meta, "create")?;
    let child = cb::call(scope, create, meta, &[], "meta.create")?;
    let looping_bot = v8::Local::<v8::Object>::try_from(looping_bot)
        .map_err(|_| cb::type_error(scope, "LoopingBot is not a constructor"))?;
    if !cb::catching(scope, |scope| child.instance_of(scope, looping_bot))? {
        let name_value = cb::get(scope, meta, "name")?;
        let name = cb::to_string(scope, name_value)?;
        return Err(Throw::Value(error_value(
            scope,
            &format!("{name} cannot run as an activity"),
        )?));
    }

    let settings = settings_bag(scope, meta, overrides, settings_constructor)?;
    let settings_key = cb::string(scope, "settings");
    cb::assign(scope, child, settings_key, settings)?;

    let activity_id = state.next_child_id.get();
    let next_id = activity_id
        .checked_add(1)
        .ok_or_else(|| cb::type_error(scope, "ScriptActivity id space exhausted"))?;
    let generation_value = v8::BigInt::new_from_u64(scope, generation);
    let child_id_value = v8::BigInt::new_from_u64(scope, activity_id);
    let log_data = activity_log_data(
        scope,
        owner,
        log,
        meta,
        generation_value.into(),
        child_id_value.into(),
    )?;
    let log_callback = build_callback(
        scope,
        activity_log_callback,
        log_data,
        "could not create ScriptActivity log callback",
    )?;
    cb::call_method(
        scope,
        child,
        "bindLog",
        &[log_callback.into()],
        "child.bindLog",
    )?;

    let finish_data = activity_finish_data(
        scope,
        owner,
        generation_value.into(),
        child_id_value.into(),
        activity_finished,
    )?;
    let finish_callback = build_callback(
        scope,
        activity_finish_callback,
        finish_data,
        "could not create ScriptActivity finish callback",
    )?;
    cb::call_method(
        scope,
        child,
        "bindFinish",
        &[finish_callback.into()],
        "child.bindFinish",
    )?;

    let (inner_resolver, inner_promise) = new_promise(scope)?;
    let data = run_data(
        scope,
        RunData {
            owner,
            child,
            log,
            meta,
            generation,
            child_id: activity_id,
            mode: "start",
            sustain,
            event_signal,
            run_manager,
            scheduler,
            script_aborted,
            activity_finished,
            bot_host,
        },
    )?;
    let on_fulfilled = build_callback(
        scope,
        activity_run_fulfilled,
        data,
        "could not create ScriptActivity completion callback",
    )?;
    let on_rejected = build_callback(
        scope,
        activity_run_rejected,
        data,
        "could not create ScriptActivity rejection callback",
    )?;

    if let Err(error) = context.start_activity(activity_id) {
        // The message each variant carries is the text ScriptActivity scripts read.
        return Err(Throw::Value(error_value(scope, &error.to_string())?));
    }
    if !context.begin_activity_start(activity_id) {
        context.clear_activity(activity_id);
        return Err(Throw::Value(error_value(
            scope,
            "An activity is already running",
        )?));
    }
    if !set_pending_resolver(scope, &state, activity_id, outer_resolver) {
        context.clear_activity(activity_id);
        return Err(cb::type_error(
            scope,
            "ScriptActivity resolver is already pending",
        ));
    }
    state.next_child_id.set(next_id);
    *state.last_owner.borrow_mut() = Some(v8::Global::new(scope, owner));
    *state.active.borrow_mut() = Some(ActiveActivity {
        generation,
        child_id: activity_id,
        owner: v8::Global::new(scope, owner),
        child: v8::Global::new(scope, child),
        log: v8::Global::new(scope, log),
        meta: v8::Global::new(scope, meta),
    });

    let attached = lineage_v8::enter_activity(scope, ActivityId(activity_id), |scope| {
        cb::catching(scope, |scope| {
            inner_promise.then2(scope, on_fulfilled, on_rejected)
        })
        .map(|_| ())
    });
    let attached = match attached {
        Ok(result) => result,
        Err(error) => Err(error),
    };
    if let Err(error) = attached {
        context.clear_activity(activity_id);
        clear_active(scope, &state, generation, activity_id, owner);
        clear_pending_resolver(&state, activity_id);
        return Err(error);
    }

    let start_call = lineage_v8::enter_activity(scope, ActivityId(activity_id), |scope| {
        let on_start = cb::get(scope, child, "onStart")?;
        if on_start.is_null_or_undefined() {
            Ok(v8::undefined(scope).into())
        } else {
            cb::call(scope, on_start, child, &[], "child.onStart")
        }
    });
    match start_call {
        Ok(Ok(value)) => settle_resolver(scope, inner_resolver, value, false)?,
        Ok(Err(Throw::Value(error))) | Err(Throw::Value(error)) => {
            settle_resolver(scope, inner_resolver, error, true)?;
        }
        Ok(Err(Throw::Terminated)) | Err(Throw::Terminated) => return Err(Throw::Terminated),
    }
    Ok(())
}

fn loop_promise<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
) -> JsResult<'s, v8::Local<'s, v8::Promise>> {
    let (resolver, promise) = new_promise(scope)?;
    match loop_inner(scope, args, resolver) {
        Ok(()) => Ok(promise),
        Err(Throw::Value(error)) => {
            settle_resolver(scope, resolver, error, true)?;
            Ok(promise)
        }
        Err(Throw::Terminated) => Err(Throw::Terminated),
    }
}

fn loop_inner<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    outer_resolver: v8::Local<'s, v8::PromiseResolver>,
) -> JsResult<'s, ()> {
    let state = state_for_scope(scope)
        .ok_or_else(|| cb::type_error(scope, "ScriptActivity state is not installed"))?;
    let undefined: v8::Local<'s, v8::Value> = v8::undefined(scope).into();
    let owner = owner_arg(scope, args.get(1))?;
    let Some(generation) = generation_arg(args.get(2)) else {
        settle_resolver(scope, outer_resolver, undefined, false)?;
        return Ok(());
    };
    let Some(context) = context_for_generation(&state, generation) else {
        settle_resolver(scope, outer_resolver, undefined, false)?;
        return Ok(());
    };
    let Some((activity_id, child, log, meta)) = active_for_owner(scope, &state, generation, owner)
    else {
        settle_resolver(scope, outer_resolver, undefined, false)?;
        return Ok(());
    };
    let snapshot = context.snapshot();
    if snapshot.state.aborted() || snapshot.activity.in_flight {
        settle_resolver(scope, outer_resolver, undefined, false)?;
        return Ok(());
    }
    let now = performance_now(scope)?;
    let next_at = cb::num(scope, snapshot.activity.next_at_ms);
    if cb::lt(scope, now, next_at)? {
        settle_resolver(scope, outer_resolver, undefined, false)?;
        return Ok(());
    }
    let bot_host = args.get(8);
    let tick = cb::get(scope, bot_host, "tickCount")?;
    let next_tick = cb::num(scope, snapshot.activity.next_tick);
    if cb::lt(scope, tick, next_tick)? {
        settle_resolver(scope, outer_resolver, undefined, false)?;
        return Ok(());
    }
    let now_ms = cb::number(scope, now)?;
    let tick_number = cb::number(scope, tick)?;
    if !context.begin_activity_loop(activity_id, tick_number, now_ms) {
        settle_resolver(scope, outer_resolver, undefined, false)?;
        return Ok(());
    }

    let sustain = args.get(3);
    let event_signal = args.get(4);
    let run_manager = args.get(5);
    let scheduler = args.get(6);
    let script_aborted = args.get(7);
    let activity_finished = args.get(9);
    let (inner_resolver, inner_promise) = new_promise(scope)?;
    let data = run_data(
        scope,
        RunData {
            owner,
            child,
            log,
            meta,
            generation,
            child_id: activity_id,
            mode: "loop",
            sustain,
            event_signal,
            run_manager,
            scheduler,
            script_aborted,
            activity_finished,
            bot_host,
        },
    )?;
    let on_fulfilled = build_callback(
        scope,
        activity_run_fulfilled,
        data,
        "could not create ScriptActivity completion callback",
    )?;
    let on_rejected = build_callback(
        scope,
        activity_run_rejected,
        data,
        "could not create ScriptActivity rejection callback",
    )?;
    if !set_pending_resolver(scope, &state, activity_id, outer_resolver) {
        context.settle_activity_start(activity_id);
        return Err(cb::type_error(
            scope,
            "ScriptActivity resolver is already pending",
        ));
    }
    let attached = lineage_v8::enter_activity(scope, ActivityId(activity_id), |scope| {
        cb::catching(scope, |scope| {
            inner_promise.then2(scope, on_fulfilled, on_rejected)
        })
        .map(|_| ())
    });
    let attached = match attached {
        Ok(result) => result,
        Err(error) => Err(error),
    };
    if let Err(error) = attached {
        context.settle_activity_start(activity_id);
        clear_pending_resolver(&state, activity_id);
        return Err(error);
    }

    let loop_call = lineage_v8::enter_activity(scope, ActivityId(activity_id), |scope| {
        cb::call_method(scope, child, "loop", &[], "child.loop")
    });
    match loop_call {
        Ok(Ok(value)) => settle_resolver(scope, inner_resolver, value, false)?,
        Ok(Err(Throw::Value(error))) | Err(Throw::Value(error)) => {
            settle_resolver(scope, inner_resolver, error, true)?;
        }
        Ok(Err(Throw::Terminated)) | Err(Throw::Terminated) => return Err(Throw::Terminated),
    }
    Ok(())
}

/// The values one ScriptActivity run callback needs, captured as its data object.
struct RunData<'s> {
    owner: v8::Local<'s, v8::Object>,
    child: v8::Local<'s, v8::Value>,
    log: v8::Local<'s, v8::Value>,
    meta: v8::Local<'s, v8::Value>,
    generation: u64,
    child_id: u64,
    mode: &'static str,
    sustain: v8::Local<'s, v8::Value>,
    event_signal: v8::Local<'s, v8::Value>,
    run_manager: v8::Local<'s, v8::Value>,
    scheduler: v8::Local<'s, v8::Value>,
    script_aborted: v8::Local<'s, v8::Value>,
    activity_finished: v8::Local<'s, v8::Value>,
    bot_host: v8::Local<'s, v8::Value>,
}

fn run_data<'s>(
    scope: &mut v8::HandleScope<'s>,
    data: RunData<'s>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let generation_value = v8::BigInt::new_from_u64(scope, data.generation).into();
    let child_id_value = v8::BigInt::new_from_u64(scope, data.child_id).into();
    let mode_value = cb::string(scope, data.mode);
    cb::object(
        scope,
        &[
            ("owner", data.owner.into()),
            ("child", data.child),
            ("log", data.log),
            ("meta", data.meta),
            ("generation", generation_value),
            ("childId", child_id_value),
            ("mode", mode_value),
            ("Sustain", data.sustain),
            ("EventSignal", data.event_signal),
            ("RunManager", data.run_manager),
            ("Scheduler", data.scheduler),
            ("ScriptAborted", data.script_aborted),
            ("ActivityFinished", data.activity_finished),
            ("BotHost", data.bot_host),
        ],
    )
}
fn activity_log_data<'s>(
    scope: &mut v8::HandleScope<'s>,
    owner: v8::Local<'s, v8::Object>,
    log: v8::Local<'s, v8::Value>,
    meta: v8::Local<'s, v8::Value>,
    generation: v8::Local<'s, v8::Value>,
    child_id: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    cb::object(
        scope,
        &[
            ("owner", owner.into()),
            ("log", log),
            ("meta", meta),
            ("generation", generation),
            ("childId", child_id),
        ],
    )
}

fn activity_finish_data<'s>(
    scope: &mut v8::HandleScope<'s>,
    owner: v8::Local<'s, v8::Object>,
    generation: v8::Local<'s, v8::Value>,
    child_id: v8::Local<'s, v8::Value>,
    activity_finished: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    cb::object(
        scope,
        &[
            ("owner", owner.into()),
            ("generation", generation),
            ("childId", child_id),
            ("ActivityFinished", activity_finished),
        ],
    )
}

fn build_callback<'s>(
    scope: &mut v8::HandleScope<'s>,
    callback: impl v8::MapFnTo<v8::FunctionCallback>,
    data: v8::Local<'s, v8::Value>,
    message: &str,
) -> JsResult<'s, v8::Local<'s, v8::Function>> {
    v8::Function::builder(callback)
        .data(data)
        .length(1)
        .build(scope)
        .ok_or_else(|| cb::type_error(scope, message))
}

fn activity_run_fulfilled<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue,
) {
    let (resolver, result) = complete_run(scope, args.data(), Some(args.get(0)), None);
    completion_callback_result(scope, resolver, result, &mut rv);
}

fn activity_run_rejected<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue,
) {
    let (resolver, result) = complete_run(scope, args.data(), None, Some(args.get(0)));
    completion_callback_result(scope, resolver, result, &mut rv);
}

fn completion_callback_result<'s>(
    scope: &mut v8::HandleScope<'s>,
    resolver: Option<v8::Local<'s, v8::PromiseResolver>>,
    result: JsResult<'s, ()>,
    rv: &mut v8::ReturnValue,
) {
    match result {
        Ok(()) | Err(Throw::Terminated) => {}
        Err(Throw::Value(error)) => {
            if let Some(resolver) = resolver {
                let _ = settle_resolver(scope, resolver, error, true);
            }
        }
    }
    rv.set(v8::undefined(scope).into());
}

fn complete_run<'s>(
    scope: &mut v8::HandleScope<'s>,
    data: v8::Local<'s, v8::Value>,
    fulfilled: Option<v8::Local<'s, v8::Value>>,
    rejected: Option<v8::Local<'s, v8::Value>>,
) -> (Option<v8::Local<'s, v8::PromiseResolver>>, JsResult<'s, ()>) {
    let state = match state_for_scope(scope) {
        Some(state) => state,
        None => {
            return (
                None,
                Err(cb::type_error(
                    scope,
                    "ScriptActivity state is not installed",
                )),
            );
        }
    };
    let child_id = match cb::get(scope, data, "childId") {
        Ok(value) => match generation_arg(value) {
            Some(child_id) => child_id,
            None => {
                return (
                    None,
                    Err(cb::type_error(
                        scope,
                        "ScriptActivity child id is unavailable",
                    )),
                );
            }
        },
        Err(error) => return (None, Err(error)),
    };
    let resolver = match take_pending_resolver(scope, &state, child_id) {
        Some(resolver) => resolver,
        None => {
            return (
                None,
                Err(cb::type_error(
                    scope,
                    "ScriptActivity resolver is unavailable",
                )),
            );
        }
    };
    let result: JsResult<'s, ()> = (|| {
        let owner_value = cb::get(scope, data, "owner")?;
        let owner = owner_arg(scope, owner_value)?;
        let generation = generation_arg(cb::get(scope, data, "generation")?);
        let Some(generation) = generation else {
            let error = error_value(scope, "Script run has ended")?;
            return reject_run(scope, resolver, error);
        };
        let Some(context) = context_for_generation(&state, generation) else {
            let error = error_value(scope, "Script run has ended")?;
            return reject_run(scope, resolver, error);
        };
        if !context.owns_activity(child_id) {
            let error = error_value(scope, "Script run has ended")?;
            return reject_run(scope, resolver, error);
        }

        let mut pending_error = rejected;
        if pending_error.is_none() {
            if let Some(loop_result) = fulfilled {
                let mode = cb::get(scope, data, "mode")?;
                if mode.strict_equals(cb::string(scope, "loop")) {
                    if let Err(error) = settle_loop_result(scope, data, loop_result, child_id) {
                        match error {
                            Throw::Value(error) => pending_error = Some(error),
                            Throw::Terminated => return Err(Throw::Terminated),
                        }
                    }
                }
            }
        }

        if let Some(error) = pending_error {
            match classify_run_error(scope, data, error, child_id) {
                Ok(true) => pending_error = Some(error),
                Ok(false) => pending_error = None,
                Err(Throw::Value(error)) => pending_error = Some(error),
                Err(Throw::Terminated) => return Err(Throw::Terminated),
            }
        }

        context.settle_activity_start(child_id);
        if context.snapshot().activity.outcome.is_some() {
            let stop_result = cb::call_method(scope, owner.into(), "stop", &[], "this.stop");
            if let Err(error) = stop_result {
                match error {
                    Throw::Value(error) => pending_error = Some(error),
                    Throw::Terminated => return Err(Throw::Terminated),
                }
            }
        }

        if pending_error.is_none() {
            let mode = cb::get(scope, data, "mode")?;
            if mode.strict_equals(cb::string(scope, "start")) {
                if active_for_owner(scope, &state, generation, owner).is_none() {
                    return settle_outer(scope, resolver, None);
                }
                // The host reads the active child's ignore list from this run-fenced handle;
                // the native guardian remains the sole random-event owner.
                state.randoms_override.set(true);
            }
        }
        settle_outer(scope, resolver, pending_error)
    })();
    (Some(resolver), result)
}

fn reject_run<'s>(
    scope: &mut v8::HandleScope<'s>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
    error: v8::Local<'s, v8::Value>,
) -> JsResult<'s, ()> {
    settle_outer(scope, resolver, Some(error))
}

fn settle_outer<'s>(
    scope: &mut v8::HandleScope<'s>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
    error: Option<v8::Local<'s, v8::Value>>,
) -> JsResult<'s, ()> {
    match error {
        Some(error) => settle_resolver(scope, resolver, error, true),
        None => {
            let undefined = v8::undefined(scope).into();
            settle_resolver(scope, resolver, undefined, false)
        }
    }
}

fn set_pending_resolver<'s>(
    scope: &mut v8::HandleScope<'s>,
    state: &ActivityState,
    child_id: u64,
    resolver: v8::Local<'s, v8::PromiseResolver>,
) -> bool {
    let mut pending = state.pending_outer.borrow_mut();
    if pending.is_some() {
        return false;
    }
    *pending = Some(PendingOuter {
        child_id,
        resolver: v8::Global::new(scope, resolver),
    });
    true
}

fn clear_pending_resolver(state: &ActivityState, child_id: u64) {
    let mut pending = state.pending_outer.borrow_mut();
    if pending
        .as_ref()
        .is_some_and(|pending| pending.child_id == child_id)
    {
        pending.take();
    }
}

fn take_pending_resolver<'s>(
    scope: &mut v8::HandleScope<'s>,
    state: &ActivityState,
    child_id: u64,
) -> Option<v8::Local<'s, v8::PromiseResolver>> {
    let pending = {
        let mut pending = state.pending_outer.borrow_mut();
        if pending
            .as_ref()
            .is_some_and(|pending| pending.child_id == child_id)
        {
            pending.take()
        } else {
            None
        }
    }?;
    Some(v8::Local::new(scope, &pending.resolver))
}

fn classify_run_error<'s>(
    scope: &mut v8::HandleScope<'s>,
    data: v8::Local<'s, v8::Value>,
    error: v8::Local<'s, v8::Value>,
    child_id: u64,
) -> JsResult<'s, bool> {
    let state = state_for_scope(scope)
        .ok_or_else(|| cb::type_error(scope, "ScriptActivity state is not installed"))?;
    let generation = generation_arg(cb::get(scope, data, "generation")?);
    let context = generation.and_then(|generation| context_for_generation(&state, generation));
    let owner_value = cb::get(scope, data, "owner")?;
    let owner = owner_arg(scope, owner_value)?;
    let script_aborted = cb::get(scope, data, "ScriptAborted")?;
    let is_aborted = instance_of(scope, error, script_aborted)?;
    if is_aborted {
        let interrupted = context
            .as_ref()
            .is_some_and(|context| context.snapshot().activity.interrupted);
        if !interrupted {
            return Ok(true);
        }
        let scheduler = cb::get(scope, data, "Scheduler")?;
        let active = cb::get(scope, scheduler, "active")?;
        if !active.is_null_or_undefined() {
            let aborted = cb::get(scope, active, "aborted")?;
            if cb::truthy(scope, aborted) {
                return Ok(true);
            }
        }
    }
    let interrupted = context
        .as_ref()
        .is_some_and(|context| context.snapshot().activity.interrupted);
    if interrupted {
        return Ok(false);
    }

    let activity_finished = cb::get(scope, data, "ActivityFinished")?;
    let finished = instance_of(scope, error, activity_finished)?;
    let kind = if finished { "finished" } else { "failed" };
    let global = scope.get_current_context().global(scope);
    let error_ctor = cb::get(scope, global.into(), "Error")?;
    let reason = if instance_of(scope, error, error_ctor)? {
        cb::get(scope, error, "message")?
    } else {
        let string_ctor = cb::get(scope, global.into(), "String")?;
        let undefined = v8::undefined(scope).into();
        cb::call(scope, string_ctor, undefined, &[error], "String")?
    };
    let reason_text = cb::to_string(scope, reason)?;
    let message = cb::string(scope, &format!("{kind}: {reason_text}"));
    let context = context.ok_or_else(|| cb::type_error(scope, "Script run has ended"))?;
    context.set_activity_outcome(child_id, kind.to_string(), reason_text, finished);
    // The native ScriptActivity stores the caller's logger as an opaque
    // callback. Invoke it with the activity object as receiver, like `this.log`.
    let log = cb::get(scope, data, "log")?;
    cb::call(scope, log, owner.into(), &[message], "this.log")?;
    Ok(false)
}

fn settle_loop_result<'s>(
    scope: &mut v8::HandleScope<'s>,
    data: v8::Local<'s, v8::Value>,
    loop_result: v8::Local<'s, v8::Value>,
    child_id: u64,
) -> JsResult<'s, ()> {
    let child = cb::get(scope, data, "child")?;
    let bot_host = cb::get(scope, data, "BotHost")?;
    let delay = if loop_result.is_number() {
        loop_result
    } else {
        cb::get(scope, child, "loopDelay")?
    };
    let override_value = cb::get(scope, child, "loopCadence")?;
    let cadence = resolve_cadence(scope, delay, override_value)?;
    let kind = cb::get(scope, cadence, "kind")?;
    let next_at = if kind.strict_equals(cb::string(scope, "time")) {
        let now = performance_now(scope)?;
        let ms = cb::get(scope, cadence, "ms")?;
        let zero = cb::num(scope, 0.0);
        let max = math_max(scope, zero, ms)?;
        cb::add(scope, now, max)?
    } else {
        cb::num(scope, 0.0)
    };
    let next_tick = if kind.strict_equals(cb::string(scope, "server-tick")) {
        let tick = cb::get(scope, bot_host, "tickCount")?;
        let ticks = cb::get(scope, cadence, "ticks")?;
        let ticks = if ticks.is_null_or_undefined() {
            cb::num(scope, 1.0)
        } else {
            ticks
        };
        let one = cb::num(scope, 1.0);
        let max = math_max(scope, one, ticks)?;
        cb::add(scope, tick, max)?
    } else {
        cb::num(scope, 0.0)
    };
    let state = state_for_scope(scope)
        .ok_or_else(|| cb::type_error(scope, "ScriptActivity state is not installed"))?;
    let generation = generation_arg(cb::get(scope, data, "generation")?);
    if let Some(context) =
        generation.and_then(|generation| context_for_generation(&state, generation))
    {
        context.settle_activity_loop(
            child_id,
            cb::number(scope, next_at)?,
            cb::number(scope, next_tick)?,
        );
    }
    Ok(())
}

fn resolve_cadence<'s>(
    scope: &mut v8::HandleScope<'s>,
    delay: v8::Local<'s, v8::Value>,
    override_value: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    if cb::truthy(scope, override_value) {
        return Ok(override_value);
    }
    let zero = cb::num(scope, 0.0);
    if cb::le(scope, delay, zero)? {
        let kind = cb::string(scope, "frame");
        return cb::object(scope, &[("kind", kind)]);
    }
    let six_hundred = cb::num(scope, 600.0);
    if delay.strict_equals(six_hundred) {
        let kind = cb::string(scope, "server-tick");
        let ticks = cb::num(scope, 1.0);
        return cb::object(scope, &[("kind", kind), ("ticks", ticks)]);
    }
    let kind = cb::string(scope, "time");
    cb::object(scope, &[("kind", kind), ("ms", delay)])
}

fn math_max<'s>(
    scope: &mut v8::HandleScope<'s>,
    left: v8::Local<'s, v8::Value>,
    right: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let global = scope.get_current_context().global(scope);
    let math = cb::get(scope, global.into(), "Math")?;
    cb::call_method(scope, math, "max", &[left, right], "Math.max")
}

fn performance_now<'s>(scope: &mut v8::HandleScope<'s>) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let global = scope.get_current_context().global(scope);
    let performance = cb::get(scope, global.into(), "performance")?;
    cb::call_method(scope, performance, "now", &[], "performance.now")
}

fn activity_log_callback<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let data = args.data();
    let id = generation_arg(
        cb::get(scope, data, "childId").unwrap_or_else(|_| v8::undefined(scope).into()),
    );
    let result = if let Some(id) = id {
        match lineage_v8::enter_activity(scope, ActivityId(id), |scope| {
            log_child(scope, data, args.get(0), id)
        }) {
            Ok(result) => result,
            Err(error) => Err(error),
        }
    } else {
        Err(cb::type_error(
            scope,
            "ScriptActivity child id is unavailable",
        ))
    };
    cb::finish(scope, rv, result);
}

fn log_child<'s>(
    scope: &mut v8::HandleScope<'s>,
    data: v8::Local<'s, v8::Value>,
    message: v8::Local<'s, v8::Value>,
    child_id: u64,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let state = state_for_scope(scope)
        .ok_or_else(|| cb::type_error(scope, "ScriptActivity state is not installed"))?;
    let generation = generation_arg(cb::get(scope, data, "generation")?);
    let Some(context) =
        generation.and_then(|generation| context_for_generation(&state, generation))
    else {
        return Ok(v8::undefined(scope).into());
    };
    if !context.owns_activity(child_id) {
        return Ok(v8::undefined(scope).into());
    }
    let owner = cb::get(scope, data, "owner")?;
    let log = cb::get(scope, data, "log")?;
    let meta = cb::get(scope, data, "meta")?;
    let name_value = cb::get(scope, meta, "name")?;
    let name = cb::to_string(scope, name_value)?;
    let message = cb::to_string(scope, message)?;
    let text = cb::string(scope, &format!("[{name}] {message}"));
    cb::call(scope, log, owner, &[text], "this.log")
}

fn activity_finish_callback<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let data = args.data();
    let id = generation_arg(
        cb::get(scope, data, "childId").unwrap_or_else(|_| v8::undefined(scope).into()),
    );
    let result = if let Some(id) = id {
        match lineage_v8::enter_activity(scope, ActivityId(id), |scope| {
            finish_child(scope, data, args.get(0), id)
        }) {
            Ok(result) => result,
            Err(error) => Err(error),
        }
    } else {
        Err(cb::type_error(
            scope,
            "ScriptActivity child id is unavailable",
        ))
    };
    cb::finish(scope, rv, result);
}

fn finish_child<'s>(
    scope: &mut v8::HandleScope<'s>,
    data: v8::Local<'s, v8::Value>,
    reason: v8::Local<'s, v8::Value>,
    child_id: u64,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let state = state_for_scope(scope)
        .ok_or_else(|| cb::type_error(scope, "ScriptActivity state is not installed"))?;
    let generation = generation_arg(cb::get(scope, data, "generation")?);
    let owner_value = cb::get(scope, data, "owner")?;
    let owner = owner_arg(scope, owner_value)?;
    let Some(context) =
        generation.and_then(|generation| context_for_generation(&state, generation))
    else {
        return Ok(v8::undefined(scope).into());
    };
    if !context.owns_activity(child_id)
        || !is_active_owner(scope, &state, generation.unwrap_or_default(), owner)
    {
        return Ok(v8::undefined(scope).into());
    }
    let reason_text = cb::to_string(scope, reason)?;
    context.set_activity_outcome(child_id, "finished".into(), reason_text, true);
    let constructor = cb::get(scope, data, "ActivityFinished")?;
    let error = cb::construct(scope, constructor, &[reason], "ActivityFinished")?;
    Err(Throw::Value(error))
}

fn child<'s>(
    scope: &mut v8::HandleScope<'s>,
    state: &ActivityState,
    args: v8::FunctionCallbackArguments<'s>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let owner = owner_arg(scope, args.get(1))?;
    let Some(generation) = generation_arg(args.get(2)) else {
        return Ok(v8::null(scope).into());
    };
    let Some((_, child, _, _)) = active_for_owner(scope, state, generation, owner) else {
        return Ok(v8::null(scope).into());
    };
    Ok(child)
}

fn outcome<'s>(
    scope: &mut v8::HandleScope<'s>,
    state: &ActivityState,
    args: v8::FunctionCallbackArguments<'s>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let owner = owner_arg(scope, args.get(1))?;
    let Some(generation) = generation_arg(args.get(2)) else {
        return Ok(v8::null(scope).into());
    };
    let Some(context) = context_for_generation(state, generation) else {
        return Ok(v8::null(scope).into());
    };
    if !is_last_owner(scope, state, owner) {
        return Ok(v8::null(scope).into());
    }
    let Some(value) = context.snapshot().activity.outcome else {
        return Ok(v8::null(scope).into());
    };
    let kind = cb::string(scope, &value.kind);
    let reason = cb::string(scope, &value.reason);
    cb::object(scope, &[("kind", kind), ("reason", reason)])
}

fn current_task<'s>(
    scope: &mut v8::HandleScope<'s>,
    state: &ActivityState,
    args: v8::FunctionCallbackArguments<'s>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let owner = owner_arg(scope, args.get(1))?;
    let Some(generation) = generation_arg(args.get(2)) else {
        return Ok(v8::null(scope).into());
    };
    let Some((_, child, _, _)) = active_for_owner(scope, state, generation, owner) else {
        return Ok(v8::null(scope).into());
    };
    let task_bot = v8::Local::<v8::Object>::try_from(args.get(3))
        .map_err(|_| cb::type_error(scope, "TaskBot is not a constructor"))?;
    if cb::catching(scope, |scope| child.instance_of(scope, task_bot))? {
        cb::get(scope, child, "activeTaskName")
    } else {
        Ok(v8::null(scope).into())
    }
}

fn stop<'s>(
    scope: &mut v8::HandleScope<'s>,
    state: &ActivityState,
    args: v8::FunctionCallbackArguments<'s>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let owner = owner_arg(scope, args.get(1))?;
    let Some(generation) = generation_arg(args.get(2)) else {
        return Ok(v8::undefined(scope).into());
    };
    let Some(context) = context_for_generation(state, generation) else {
        return Ok(v8::undefined(scope).into());
    };
    if context.snapshot().activity.in_flight {
        let reason = cb::string(scope, "activity stopped");
        interrupt_impl(scope, state, owner, generation, reason)?;
        return Ok(v8::undefined(scope).into());
    }
    stop_child(
        scope,
        state,
        owner,
        generation,
        args.get(3),
        args.get(4),
        args.get(5),
    )?;
    Ok(v8::undefined(scope).into())
}

fn stop_child<'s>(
    scope: &mut v8::HandleScope<'s>,
    state: &ActivityState,
    owner: v8::Local<'s, v8::Object>,
    generation: u64,
    sustain: v8::Local<'s, v8::Value>,
    event_signal: v8::Local<'s, v8::Value>,
    run_manager: v8::Local<'s, v8::Value>,
) -> JsResult<'s, ()> {
    let Some(context) = context_for_generation(state, generation) else {
        return Ok(());
    };
    let Some((child_id, _, _, _)) = active_for_owner(scope, state, generation, owner) else {
        return Ok(());
    };
    let active = state.active.borrow_mut().take();
    let Some(active) = active else {
        return Ok(());
    };
    let child = v8::Local::new(scope, &active.child);
    let on_stop_result = lineage_v8::enter_activity(scope, ActivityId(child_id), |scope| {
        let on_stop = cb::get(scope, child, "onStop")?;
        if on_stop.is_null_or_undefined() {
            Ok(v8::undefined(scope).into())
        } else {
            cb::call(scope, on_stop, child, &[], "child.onStop")
        }
    });
    let on_stop_result = match on_stop_result {
        Ok(result) => result,
        Err(error) => Err(error),
    };
    let cleanup_result = match on_stop_result {
        Err(Throw::Terminated) => Err(Throw::Terminated),
        on_stop => {
            let cleanup = lineage_v8::enter_activity(scope, ActivityId(child_id), |scope| {
                cleanup_child(scope, sustain, event_signal, run_manager, child)
            });
            let cleanup = match cleanup {
                Ok(result) => result,
                Err(error) => Err(error),
            };
            match cleanup {
                Err(error) => Err(error),
                Ok(()) => {
                    state.randoms_override.set(true);
                    on_stop.map(|_| ())
                }
            }
        }
    };
    context.clear_activity(child_id);
    state.active.borrow_mut().take();
    match cleanup_result {
        Ok(()) => Ok(()),
        Err(error) => Err(error),
    }
}

fn cleanup_child<'s>(
    scope: &mut v8::HandleScope<'s>,
    sustain: v8::Local<'s, v8::Value>,
    event_signal: v8::Local<'s, v8::Value>,
    run_manager: v8::Local<'s, v8::Value>,
    child: v8::Local<'s, v8::Value>,
) -> JsResult<'s, ()> {
    cb::call_method(
        scope,
        child,
        "disposeSubscriptions",
        &[],
        "child.disposeSubscriptions",
    )?;
    let null = v8::null(scope).into();
    cb::call_method(scope, sustain, "set", &[null], "Sustain.set")?;
    cb::call_method(
        scope,
        event_signal,
        "setInterrupt",
        &[null],
        "EventSignal.setInterrupt",
    )?;
    cb::call_method(
        scope,
        run_manager,
        "override",
        &[null],
        "RunManager.override",
    )?;
    Ok(())
}

fn interrupt<'s>(
    scope: &mut v8::HandleScope<'s>,
    state: &ActivityState,
    args: v8::FunctionCallbackArguments<'s>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let owner = owner_arg(scope, args.get(1))?;
    let Some(generation) = generation_arg(args.get(2)) else {
        return Ok(v8::undefined(scope).into());
    };
    let reason = args.get(3);
    interrupt_impl(scope, state, owner, generation, reason)?;
    Ok(v8::undefined(scope).into())
}

fn interrupt_impl<'s>(
    scope: &mut v8::HandleScope<'s>,
    state: &ActivityState,
    owner: v8::Local<'s, v8::Object>,
    generation: u64,
    reason: v8::Local<'s, v8::Value>,
) -> JsResult<'s, ()> {
    let Some(context) = context_for_generation(state, generation) else {
        return Ok(());
    };
    let Some(child_id) = active_child_id(scope, state, generation, owner) else {
        *state.last_owner.borrow_mut() = Some(v8::Global::new(scope, owner));
        let reason = cb::to_string(scope, reason)?;
        context.interrupt_activity(None, reason);
        return Ok(());
    };
    let in_flight = context.snapshot().activity.in_flight;
    let reason_text = cb::to_string(scope, reason)?;
    context.interrupt_activity(Some(child_id), reason_text);
    if in_flight {
        crate::runtime::lifecycle_v8::abort_activity_waiters(scope, generation, child_id)?;
    } else {
        cb::call_method(scope, owner.into(), "stop", &[], "this.stop")?;
    }
    Ok(())
}

fn ignored_randoms<'s>(
    scope: &mut v8::HandleScope<'s>,
    state: &ActivityState,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    if !state.randoms_override.get() {
        return Ok(v8::undefined(scope).into());
    }
    let Some(context) = state.context.borrow().as_ref().map(Arc::clone) else {
        return Ok(v8::Array::new(scope, 0).into());
    };
    let active = state.active.borrow();
    let Some(active) = active.as_ref() else {
        return Ok(v8::Array::new(scope, 0).into());
    };
    if active.generation != context.snapshot().generation
        || context.state().aborted()
        || !context.owns_activity(active.child_id)
    {
        return Ok(v8::Array::new(scope, 0).into());
    }
    let child_id = active.child_id;
    let child = v8::Local::new(scope, &active.child);
    let result = lineage_v8::enter_activity(scope, ActivityId(child_id), |scope| {
        cb::call_method(scope, child, "ignoredRandoms", &[], "child.ignoredRandoms")
    });
    match result {
        Ok(Ok(value)) if value.is_array() => Ok(value),
        Ok(Ok(_)) | Ok(Err(Throw::Value(_))) | Err(Throw::Value(_)) => {
            Ok(v8::Array::new(scope, 0).into())
        }
        Ok(Err(Throw::Terminated)) | Err(Throw::Terminated) => Err(Throw::Terminated),
    }
}

/// The active child's id, child object, log function and meta object.
type ActiveChild<'s> = (
    u64,
    v8::Local<'s, v8::Value>,
    v8::Local<'s, v8::Value>,
    v8::Local<'s, v8::Value>,
);

fn active_for_owner<'s>(
    scope: &mut v8::HandleScope<'s>,
    state: &ActivityState,
    generation: u64,
    owner: v8::Local<'s, v8::Object>,
) -> Option<ActiveChild<'s>> {
    let context = context_for_generation(state, generation)?;
    let active = state.active.borrow();
    let active = active.as_ref()?;
    if active.generation != generation
        || !v8::Local::new(scope, &active.owner).strict_equals(owner.into())
        || !context.owns_activity(active.child_id)
    {
        return None;
    }
    Some((
        active.child_id,
        v8::Local::new(scope, &active.child),
        v8::Local::new(scope, &active.log),
        v8::Local::new(scope, &active.meta),
    ))
}

fn active_child_id<'s>(
    scope: &mut v8::HandleScope<'s>,
    state: &ActivityState,
    generation: u64,
    owner: v8::Local<'s, v8::Object>,
) -> Option<u64> {
    active_for_owner(scope, state, generation, owner).map(|active| active.0)
}

fn is_active_owner<'s>(
    scope: &mut v8::HandleScope<'s>,
    state: &ActivityState,
    generation: u64,
    owner: v8::Local<'s, v8::Object>,
) -> bool {
    active_child_id(scope, state, generation, owner).is_some()
}

fn is_last_owner<'s>(
    scope: &mut v8::HandleScope<'s>,
    state: &ActivityState,
    owner: v8::Local<'s, v8::Object>,
) -> bool {
    state
        .last_owner
        .borrow()
        .as_ref()
        .is_some_and(|current| v8::Local::new(scope, current).strict_equals(owner.into()))
}

fn clear_active(
    scope: &mut v8::HandleScope,
    state: &ActivityState,
    generation: u64,
    child_id: u64,
    owner: v8::Local<v8::Object>,
) {
    let matches = state.active.borrow().as_ref().is_some_and(|active| {
        active.generation == generation
            && active.child_id == child_id
            && v8::Local::new(scope, &active.owner).strict_equals(owner.into())
    });
    if matches {
        state.active.borrow_mut().take();
    }
}

fn settings_bag<'s>(
    scope: &mut v8::HandleScope<'s>,
    meta: v8::Local<'s, v8::Value>,
    overrides: v8::Local<'s, v8::Value>,
    constructor: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let schema = cb::get(scope, meta, "settingsSchema")?;
    let schema = if schema.is_null_or_undefined() {
        v8::Object::new(scope).into()
    } else {
        schema
    };
    let global = scope.get_current_context().global(scope);
    let object_ctor = cb::get(scope, global.into(), "Object")?;
    let entries_fn = cb::get(scope, object_ctor, "entries")?;
    let entries = cb::call(scope, entries_fn, object_ctor, &[schema], "Object.entries")?;
    let mapper = v8::Function::builder(settings_default_entry)
        .build(scope)
        .ok_or_else(|| cb::type_error(scope, "could not create settings default mapper"))?;
    let mapped = cb::call_method(scope, entries, "map", &[mapper.into()], "entries.map")?;
    let from_entries = cb::get(scope, object_ctor, "fromEntries")?;
    let defaults = cb::call(
        scope,
        from_entries,
        object_ctor,
        &[mapped],
        "Object.fromEntries",
    )?;
    let values = v8::Object::new(scope);
    spread_copy(scope, values, defaults)?;
    spread_copy(scope, values, overrides)?;
    cb::construct(scope, constructor, &[values.into()], "SettingsBag")
}

fn settings_default_entry<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = (|| {
        let pair = args.get(0);
        let key = cb::get_index(scope, pair, 0)?;
        let setting = cb::get_index(scope, pair, 1)?;
        let default = cb::get(scope, setting, "default")?;
        let row = v8::Array::new(scope, 2);
        if !row.set_index(scope, 0, key).unwrap_or(false)
            || !row.set_index(scope, 1, default).unwrap_or(false)
        {
            return Err(cb::type_error(
                scope,
                "could not create settings default entry",
            ));
        }
        Ok(row.into())
    })();
    cb::finish(scope, rv, result);
}

fn spread_copy<'s>(
    scope: &mut v8::HandleScope<'s>,
    target: v8::Local<'s, v8::Object>,
    source: v8::Local<'s, v8::Value>,
) -> JsResult<'s, ()> {
    if source.is_null_or_undefined() {
        return Ok(());
    }
    let source = cb::catching(scope, |scope| source.to_object(scope))?;
    let global = scope.get_current_context().global(scope);
    let reflect = cb::get(scope, global.into(), "Reflect")?;
    let own_keys = cb::get(scope, reflect, "ownKeys")?;
    let keys = cb::call(
        scope,
        own_keys,
        reflect,
        &[source.into()],
        "Reflect.ownKeys",
    )?;
    let keys = v8::Local::<v8::Array>::try_from(keys)
        .map_err(|_| cb::type_error(scope, "Reflect.ownKeys did not return an array"))?;
    let object_ctor = cb::get(scope, global.into(), "Object")?;
    let descriptor_fn = cb::get(scope, object_ctor, "getOwnPropertyDescriptor")?;
    for index in 0..keys.length() {
        let key = cb::get_index(scope, keys.into(), index)?;
        let key_name = v8::Local::<v8::Name>::try_from(key)
            .map_err(|_| cb::type_error(scope, "settings spread key is not a property name"))?;
        let descriptor = cb::call(
            scope,
            descriptor_fn,
            object_ctor,
            &[source.into(), key],
            "Object.getOwnPropertyDescriptor",
        )?;
        if descriptor.is_null_or_undefined() {
            continue;
        }
        let enumerable = cb::get(scope, descriptor, "enumerable")?;
        if !cb::truthy(scope, enumerable) {
            continue;
        }
        let value = cb::catching(scope, |scope| source.get(scope, key))?;
        let created = cb::catching(scope, |scope| {
            target.create_data_property(scope, key_name, value)
        })?;
        if !created {
            return Err(cb::type_error(scope, "could not copy settings property"));
        }
    }
    Ok(())
}

#[cfg(all(test, feature = "memory-profile"))]
mod v8_tests {
    use std::process::Command;

    use rustyscript::deno_core::v8;
    use rustyscript::{Runtime, RuntimeOptions};
    use serde_json::json;

    use super::*;

    const FAST_CHILD: &str = "SCRIPT_ACTIVITY_DATA_FAST_TEST_CHILD";
    const FAST_TEST: &str = "activity_v8::v8_tests::activity_data_objects_have_fast_properties";
    const HEAP_CHILD: &str = "SCRIPT_ACTIVITY_DATA_HEAP_TEST_CHILD";
    const HEAP_TEST: &str =
        "activity_v8::v8_tests::repeated_activity_data_objects_have_bounded_post_gc_heap";

    const SETUP: &str = r#"
globalThis.__ActivityTestSettingsBag = class {
    constructor(values) { return values; }
};
globalThis.__activityTestMeta = {
    name: 'V8 activity data',
    settingsSchema: {
        food: { default: 'Lobster' },
        enabled: { default: false },
    },
};
globalThis.__activityTestOverrides = { food: 'Shark' };
"#;

    fn setup() -> Runtime {
        crate::runtime::platform::bootstrap_test_v8_platform();
        let mut runtime = Runtime::new(RuntimeOptions::default()).unwrap();
        cb::install_ops(&mut runtime).unwrap();
        cb::install(
            &mut runtime,
            "__rs2b0t_activity_test_build_data",
            build_activity_data,
        )
        .unwrap();
        runtime.eval::<()>(SETUP).unwrap();
        runtime
    }

    fn in_child(marker: &str, test_name: &str) -> bool {
        if std::env::var_os(marker).is_some() {
            return true;
        }
        let output = Command::new(std::env::current_exe().expect("test executable"))
            .args(["--exact", test_name, "--nocapture"])
            .env(marker, "1")
            .output()
            .expect("run isolated ScriptActivity V8 test");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            output.status.success() && stdout.contains("1 passed"),
            "isolated ScriptActivity V8 test failed\nstdout: {stdout}\nstderr: {stderr}"
        );
        false
    }

    fn build_activity_data<'s>(
        scope: &mut v8::HandleScope<'s>,
        _args: v8::FunctionCallbackArguments<'s>,
        rv: v8::ReturnValue,
    ) {
        let result = (|| -> JsResult<'s, v8::Local<'s, v8::Value>> {
            let global = scope.get_current_context().global(scope);
            let meta = cb::get(scope, global.into(), "__activityTestMeta")?;
            let overrides = cb::get(scope, global.into(), "__activityTestOverrides")?;
            let settings_constructor = cb::get(scope, global.into(), "__ActivityTestSettingsBag")?;
            let owner = v8::Object::new(scope);
            let child = v8::Object::new(scope);
            let log = v8::undefined(scope).into();
            let generation = v8::BigInt::new_from_u64(scope, 71).into();
            let child_id = v8::BigInt::new_from_u64(scope, 3).into();
            let undefined = v8::undefined(scope).into();

            let log_data = activity_log_data(scope, owner, log, meta, generation, child_id)?;
            let finish_data = activity_finish_data(scope, owner, generation, child_id, undefined)?;
            let run_data = run_data(
                scope,
                RunData {
                    owner,
                    child: child.into(),
                    log,
                    meta,
                    generation: 71,
                    child_id: 3,
                    mode: "loop",
                    sustain: undefined,
                    event_signal: undefined,
                    run_manager: undefined,
                    scheduler: undefined,
                    script_aborted: undefined,
                    activity_finished: undefined,
                    bot_host: undefined,
                },
            )?;

            let schema = cb::get(scope, meta, "settingsSchema")?;
            let setting = cb::get(scope, schema, "food")?;
            let key = cb::string(scope, "food");
            let pair = v8::Array::new_with_elements(scope, &[key, setting]);
            let mapper = v8::Function::builder(settings_default_entry)
                .build(scope)
                .ok_or_else(|| cb::type_error(scope, "could not create settings row test"))?;
            let row = cb::call(
                scope,
                mapper.into(),
                undefined,
                &[pair.into()],
                "settings row test",
            )?;
            let settings = settings_bag(scope, meta, overrides, settings_constructor)?;
            Ok(v8::Array::new_with_elements(
                scope,
                &[log_data, finish_data, run_data, row, settings],
            )
            .into())
        })();
        cb::finish(scope, rv, result);
    }

    fn collected_used_heap(runtime: &mut Runtime) -> usize {
        let isolate = runtime.deno_runtime().v8_isolate();
        isolate.low_memory_notification();
        isolate.get_heap_statistics().used_heap_size()
    }

    #[test]
    fn activity_data_objects_have_fast_properties() {
        if !in_child(FAST_CHILD, FAST_TEST) {
            return;
        }
        v8::V8::set_flags_from_string("--allow-natives-syntax");
        let mut runtime = setup();
        let facts: serde_json::Value = runtime
            .eval(
                r#"
(() => {
    const hasFast = new Function('value', 'return %HasFastProperties(value)');
    return __rs2b0t_activity_test_build_data().map((value) => ({
        array: Array.isArray(value),
        fast: hasFast(value),
        keys: Object.keys(value),
    }));
})()
"#,
            )
            .unwrap();
        assert_eq!(
            facts,
            json!([
                {
                    "array": false,
                    "fast": true,
                    "keys": ["owner", "log", "meta", "generation", "childId"]
                },
                {
                    "array": false,
                    "fast": true,
                    "keys": ["owner", "generation", "childId", "ActivityFinished"]
                },
                {
                    "array": false,
                    "fast": true,
                    "keys": [
                        "owner", "child", "log", "meta", "generation", "childId", "mode",
                        "Sustain", "EventSignal", "RunManager", "Scheduler", "ScriptAborted",
                        "ActivityFinished", "BotHost"
                    ]
                },
                { "array": true, "fast": true, "keys": ["0", "1"] },
                { "array": false, "fast": true, "keys": ["food", "enabled"] }
            ]),
            "native ScriptActivity callback data and settings objects retain fixed shapes"
        );
    }

    #[test]
    fn repeated_activity_data_objects_have_bounded_post_gc_heap() {
        if !in_child(HEAP_CHILD, HEAP_TEST) {
            return;
        }
        let mut runtime = setup();
        runtime
            .eval::<()>(
                "for (let index = 0; index < 1000; index++) __rs2b0t_activity_test_build_data()",
            )
            .unwrap();
        let baseline = collected_used_heap(&mut runtime);
        runtime
            .eval::<()>(
                "for (let index = 0; index < 25000; index++) __rs2b0t_activity_test_build_data()",
            )
            .unwrap();
        let after = collected_used_heap(&mut runtime);
        let growth = after.saturating_sub(baseline);
        const LIMIT: usize = 64 * 1024;
        println!(
            "ScriptActivity callback/settings post-GC V8 heap baseline={baseline} after={after} growth={growth} limit={LIMIT}"
        );
        assert!(
            growth <= LIMIT,
            "transient ScriptActivity callback/settings objects grew post-GC V8 heap by {growth} bytes; limit {LIMIT}"
        );
    }
}
