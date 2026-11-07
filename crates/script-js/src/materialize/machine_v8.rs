//! V8 marshalling for the step-machine host (`crate::machine`).
//!
//! - `__rs2b0t_machine_start(family, args, hooks)` holds the family's
//!   declared callbacks off `hooks`, begins the machine and returns its
//!   start envelope.
//! - `__rs2b0t_machine_take(handle)` is the parked wait's poll
//!   (`undefined` while the machine runs).
//! - [`step`] is the isolate thread's per-tick step, with script callbacks
//!   invoked through the one callback path (`callback_v8`), each followed
//!   by a microtask checkpoint; a kick polls the callback promise without
//!   draining unrelated queued microtasks. A promise still pending is
//!   held and its state polled.
//! - [`resume_callbacks`] runs after the tick's pump; [`settle_waits`]
//!   then resolves awaits for rows that ended in the same tick
//!   (`__rs2b0t_settle_machines`).
//!
//! Nothing here sends to the host: machine ops join the tick's
//! InteractReq batch in Rust.

use super::callback_v8::{self, Callback, HeldCallback, Throw};
use crate::machine::{self, Called, Hook, Outcome, Pending, Reply, Started, Take, Thrown};
use rustyscript::deno_core::error::JsError;
use rustyscript::deno_core::serde_v8;
use rustyscript::{json_args, Runtime};
use serde_json::Value;
use std::cell::RefCell;
use std::rc::Rc;

pub(crate) fn install(runtime: &mut Runtime) -> Result<(), String> {
    // An existing cache stays; it holds shapes only, never a materialized
    // value.
    runtime
        .deno_runtime()
        .v8_isolate()
        .set_slot(Rc::new(RefCell::new(ValueShapes::default())));
    callback_v8::install(runtime, "__rs2b0t_machine_start", start_callback)?;
    #[cfg(feature = "test-hooks")]
    callback_v8::install(runtime, "__rs2b0t_test_machine_host", test_host_callback)?;
    callback_v8::install(runtime, "__rs2b0t_machine_take", take_callback)
}

/// Test seam: the machine host as JSON text, for the seat page tests.
#[cfg(feature = "test-hooks")]
fn test_host_callback<'s>(
    scope: &mut v8::HandleScope<'s>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue,
) {
    let text = machine::test_host_report().to_string();
    if let Some(value) = v8::String::new(scope, &text) {
        rv.set(value.into());
    }
}

/// Step every live machine once, before the tick's other JS. `claimed`
/// is join's claim on the tick: once it holds, no further callback runs.
pub(crate) fn step(runtime: &mut Runtime, claimed: &dyn Fn() -> bool) {
    machine::step(&mut RuntimeJs { runtime, claimed });
}
/// Re-evaluate opted-in machines against a changed snapshot without advancing
/// the tick or resetting their callback budget.
pub(crate) fn snapshot_step(
    runtime: &mut Runtime,
    tick: u64,
    evidence_sequence: u64,
    claimed: &dyn Fn() -> bool,
) {
    machine::snapshot_step(&mut RuntimeJs { runtime, claimed }, tick, evidence_sequence);
}

/// Step only the native rows that the posted seat page changed and no pass
/// has read, keeping the callback budget. Returns the rows it stepped.
pub(crate) fn seat_step(runtime: &mut Runtime, claimed: &dyn Fn() -> bool) -> machine::SteppedRows {
    machine::seat_step(&mut RuntimeJs { runtime, claimed })
}

/// After a seat page step and its drain, resume the stepped rows whose
/// callback promise settled. No other row resumes.
pub(crate) fn resume_callbacks_seat(
    runtime: &mut Runtime,
    stepped: &machine::SteppedRows,
    claimed: &dyn Fn() -> bool,
) {
    machine::resume_seat(&mut RuntimeJs { runtime, claimed }, stepped);
}

/// After the tick's pump, resume rows whose callback promise settled.
/// Kept separate from [`settle_waits`] because callbacks are not the script
/// loop continuation and must not inherit its interrupt stage.
pub(crate) fn resume_callbacks(runtime: &mut Runtime, claimed: &dyn Fn() -> bool) {
    machine::resume(&mut RuntimeJs { runtime, claimed });
}

/// Settle awaits for machine rows that ended during this tick.
pub(crate) fn settle_waits(
    runtime: &mut Runtime,
    claimed: &dyn Fn() -> bool,
) -> Result<(), rustyscript::Error> {
    if machine::any_settled() && !claimed() {
        super::lineage_v8::call_function_immediate::<()>(
            runtime,
            None,
            "__rs2b0t_settle_machines",
            json_args!(),
        )?;
    }
    Ok(())
}

/// Settle only the awaits of outcomes whose seat terminal is above
/// `release_floor`: the terminals the seat published after the last accepted
/// seat answer dispatch. The handle list lives on the stack; the JS array of
/// it exists only when a terminal qualifies (once per ended request, not per
/// wake), and the shim re-parks every other machine wait without taking it.
pub(crate) fn settle_waits_seat(
    runtime: &mut Runtime,
    release_floor: u64,
    claimed: &dyn Fn() -> bool,
) -> Result<(), rustyscript::Error> {
    let released = machine::settled_above(release_floor);
    let handles = released.as_slice();
    if !handles.is_empty() && !claimed() {
        super::lineage_v8::call_function_immediate::<()>(
            runtime,
            None,
            "__rs2b0t_settle_machines",
            &(handles,),
        )?;
    }
    Ok(())
}

fn start_callback<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    super::lineage_v8::assert_guarded(scope);
    let parent = match super::lineage_v8::resolve_lineage(scope) {
        super::lineage_v8::Lineage::Root => None,
        super::lineage_v8::Lineage::Token(handle) => Some(handle),
        super::lineage_v8::Lineage::Foreign => {
            let reason = callback_v8::string(scope, "lineage carrier is not installed");
            let envelope = envelope(scope, "refused", "reason", reason);
            return callback_v8::finish(scope, rv, envelope);
        }
    };
    let family = args.get(0).to_rust_string_lossy(scope);
    let hooks = match hold_hooks(scope, &family, args.get(2)) {
        Ok(hooks) => hooks,
        Err(throw) => return callback_v8::finish(scope, rv, Err(throw)),
    };
    let at = js_queue_len(scope);
    let started = match serde_v8::from_v8::<Value>(scope, args.get(1)) {
        Ok(value) => machine::start_with_parent(&family, value, hooks, at, parent),
        Err(e) => Started::Refused(format!("{family} arguments: {e}")),
    };
    if let Started::Running(handle) = &started {
        if machine::kick_on_start(&family) {
            machine::kick(*handle, &mut ScopeJs { scope });
        }
    }
    let envelope = match started {
        Started::Running(handle) => {
            let handle = callback_v8::num(scope, handle as f64);
            envelope(scope, "running", "handle", handle)
        }
        Started::Settled(outcome) => outcome_envelope(scope, outcome),
        Started::Refused(reason) => {
            let reason = callback_v8::string(scope, &reason);
            envelope(scope, "refused", "reason", reason)
        }
    };
    callback_v8::finish(scope, rv, envelope);
}

/// `hooks[name]` for each declared callback, read once now and called as
/// `hooks.name(...)`. A family without callbacks never reads `hooks`.
fn hold_hooks<'s>(
    scope: &mut v8::HandleScope<'s>,
    family: &str,
    hooks: v8::Local<'s, v8::Value>,
) -> Result<Vec<Hook>, Throw<'s>> {
    let names = machine::callbacks_of(family).unwrap_or_default();
    let mut held = Vec::with_capacity(names.len());
    for &name in names {
        let func = callback_v8::get(scope, hooks, name)?;
        held.push(Hook {
            present: !func.is_null_or_undefined(),
            callback: Callback::method(func, hooks, name).hold(scope),
        });
    }
    Ok(held)
}

fn take_callback<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    super::lineage_v8::assert_guarded(scope);
    let handle = args.get(0).number_value(scope).unwrap_or(0.0) as machine::Handle;
    if let Take::Settled(outcome) = machine::take(handle) {
        let envelope = outcome_envelope(scope, outcome);
        callback_v8::finish(scope, rv, envelope);
    }
}

fn outcome_envelope<'s>(
    scope: &mut v8::HandleScope<'s>,
    outcome: Outcome,
) -> callback_v8::JsResult<'s, v8::Local<'s, v8::Value>> {
    match outcome {
        Outcome::Done(value) => {
            let value = value_to_v8(scope, &value, 0)
                .map_err(|e| callback_v8::type_error(scope, &format!("machine value: {e}")))?;
            envelope(scope, "done", "value", value)
        }
        Outcome::Refused(reason) => {
            let reason = callback_v8::string(scope, &reason);
            envelope(scope, "refused", "reason", reason)
        }
        Outcome::Failed(thrown) => {
            let error = match thrown.value() {
                Some(value) => v8::Local::new(scope, value),
                None => {
                    let message = v8::String::new(scope, thrown.message())
                        .unwrap_or_else(|| v8::String::empty(scope));
                    v8::Exception::error(scope, message)
                }
            };
            envelope(scope, "failed", "error", error)
        }
        Outcome::Aborted(why) => {
            let reason = callback_v8::string(scope, why.as_str());
            envelope(scope, "aborted", "reason", reason)
        }
    }
}

/// `{ kind, [field]: value }`.
fn envelope<'s>(
    scope: &mut v8::HandleScope<'s>,
    kind: &str,
    field: &str,
    value: v8::Local<'s, v8::Value>,
) -> callback_v8::JsResult<'s, v8::Local<'s, v8::Value>> {
    let kind = callback_v8::string(scope, kind);
    callback_v8::object(scope, &[("kind", kind), (field, value)])
}

/// Most distinct object shapes one isolate caches for machine values.
const MAX_VALUE_SHAPES: usize = 64;
/// Most keys an object may have to get a cached shape.
const MAX_SHAPE_KEYS: usize = 32;
/// Deepest nesting built natively; a deeper value goes through `serde_v8`.
const MAX_VALUE_DEPTH: usize = 8;

/// Per-isolate templates that give a machine's `Done` value objects fast
/// properties.
///
/// `serde_v8` builds every JSON object with a null prototype, which V8 keeps in
/// dictionary mode. A template installs the object's complete key list once;
/// V8 migrates the cached instantiation to fast properties and copies that
/// layout into every instance, so writing the values afterwards updates
/// existing fields and never adds one (the same layout rule as the entity
/// rows). The cache holds the templates only: no materialized value, bounded
/// by [`MAX_VALUE_SHAPES`], dropped with the isolate.
#[derive(Default)]
struct ValueShapes {
    shapes: Vec<ValueShape>,
}

struct ValueShape {
    keys: Box<[Box<str>]>,
    template: v8::Global<v8::ObjectTemplate>,
}

impl ValueShapes {
    fn find(&self, map: &serde_json::Map<String, Value>) -> Option<&ValueShape> {
        self.shapes.iter().find(|shape| {
            shape.keys.len() == map.len()
                && shape.keys.iter().zip(map.keys()).all(|(a, b)| **a == **b)
        })
    }
}

/// The template for `map`'s ordered key list, built on first use. `None` when
/// the object has no shape worth caching (empty, too many keys) or the cache
/// is full: the caller then builds the object from `v8::Object::new`, which
/// still has fast properties and `Object.prototype`.
fn shape_template<'s>(
    scope: &mut v8::HandleScope<'s>,
    map: &serde_json::Map<String, Value>,
) -> Option<v8::Local<'s, v8::ObjectTemplate>> {
    if map.is_empty() || map.len() > MAX_SHAPE_KEYS {
        return None;
    }
    let shapes = scope.get_slot::<Rc<RefCell<ValueShapes>>>()?.clone();
    if let Some(shape) = shapes.borrow().find(map) {
        return Some(v8::Local::new(scope, &shape.template));
    }
    if shapes.borrow().shapes.len() >= MAX_VALUE_SHAPES {
        return None;
    }
    let template = v8::ObjectTemplate::new(scope);
    let placeholder: v8::Local<v8::Data> = v8::undefined(scope).into();
    for key in map.keys() {
        let name =
            v8::String::new_from_utf8(scope, key.as_bytes(), v8::NewStringType::Internalized)?;
        template.set(name.into(), placeholder);
    }
    shapes.borrow_mut().shapes.push(ValueShape {
        keys: map.keys().map(|key| key.as_str().into()).collect(),
        template: v8::Global::new(scope, template),
    });
    Some(template)
}

/// `value` as a V8 value, the same data `serde_v8::to_v8` builds but with
/// objects that carry fast properties (see [`ValueShapes`]) and
/// `Object.prototype`, also when the shape cache is full or the object is
/// wide. Numbers outside `i32`, and values nested deeper than
/// [`MAX_VALUE_DEPTH`], go through `serde_v8`.
fn value_to_v8<'s>(
    scope: &mut v8::HandleScope<'s>,
    value: &Value,
    depth: usize,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let fallback =
        |scope: &mut v8::HandleScope<'s>| serde_v8::to_v8(scope, value).map_err(|e| e.to_string());
    Ok(match value {
        Value::Null => v8::null(scope).into(),
        Value::Bool(on) => v8::Boolean::new(scope, *on).into(),
        Value::Number(number) => match number.as_i64().and_then(|n| i32::try_from(n).ok()) {
            Some(n) => v8::Integer::new(scope, n).into(),
            None => return fallback(scope),
        },
        Value::String(text) => match v8::String::new(scope, text) {
            Some(text) => text.into(),
            None => return fallback(scope),
        },
        _ if depth >= MAX_VALUE_DEPTH => return fallback(scope),
        Value::Array(items) => {
            let mut elements = Vec::with_capacity(items.len());
            for item in items {
                elements.push(value_to_v8(scope, item, depth + 1)?);
            }
            v8::Array::new_with_elements(scope, &elements).into()
        }
        Value::Object(map) if map.is_empty() => v8::Object::new(scope).into(),
        Value::Object(map) => {
            let object = match shape_template(scope, map) {
                Some(template) => template
                    .new_instance(scope)
                    .ok_or_else(|| "object instantiation failed".to_string())?,
                None => v8::Object::new(scope),
            };
            for (key, item) in map {
                let name = v8::String::new_from_utf8(
                    scope,
                    key.as_bytes(),
                    v8::NewStringType::Internalized,
                )
                .ok_or_else(|| "key allocation failed".to_string())?;
                let item = value_to_v8(scope, item, depth + 1)?;
                object.create_data_property(scope, name.into(), item);
            }
            object.into()
        }
    })
}

/// Length of `__rs2b0t_host.interact`: where machine ops sit in the tick
/// batch relative to the rows JS queued.
fn js_queue_len(scope: &mut v8::HandleScope) -> usize {
    let context = scope.get_current_context();
    let global = context.global(scope);
    let Some(host_key) = v8::String::new(scope, "__rs2b0t_host") else {
        return 0;
    };
    let Some(host) = global
        .get(scope, host_key.into())
        .and_then(|host| host.to_object(scope))
    else {
        return 0;
    };
    let Some(interact_key) = v8::String::new(scope, "interact") else {
        return 0;
    };
    let Some(rows) = host
        .get(scope, interact_key.into())
        .and_then(|rows| v8::Local::<v8::Object>::try_from(rows).ok())
    else {
        return 0;
    };
    let Some(length_key) = v8::String::new(scope, "length") else {
        return 0;
    };
    // The guarded append-only queue is a Proxy, not a V8 Array handle.
    rows.get(scope, length_key.into())
        .filter(|length| length.is_number())
        .and_then(|length| length.number_value(scope))
        .filter(|length| (0.0..=u32::MAX as f64).contains(length) && length.fract() == 0.0)
        .map_or(0, |length| length as usize)
}

/// Drive a just-started row from the start callback's own scope.
struct ScopeJs<'s, 'i> {
    scope: &'s mut v8::HandleScope<'i>,
}

impl machine::Js for ScopeJs<'_, '_> {
    fn queue_len(&mut self) -> usize {
        js_queue_len(self.scope)
    }

    fn call(&mut self, _hook: Option<&HeldCallback>, _args: &[Value]) -> Called {
        // V8 machine calls must carry their owning row.
        Called::Terminated
    }

    fn call_for(
        &mut self,
        handle: machine::Handle,
        hook: Option<&HeldCallback>,
        args: &[Value],
    ) -> Called {
        // Nested in the caller's JS: draining the isolate queue here would
        // run unrelated microtasks before the caller continues (M-187).
        call_hook(self.scope, handle, hook, args, false)
    }

    fn poll(&mut self, pending: &Pending) -> Option<Reply> {
        poll_promise(self.scope, pending)
    }

    /// A start runs inside the caller's own script call: join's claim is
    /// checked when that call returns. A callback terminated inside the
    /// kick ends the kick itself ([`machine::Called::Terminated`]).
    fn claimed(&mut self) -> bool {
        false
    }

    fn game_messages_mark(&mut self) -> f64 {
        super::chatbox_v8::native_mark(self.scope)
    }
}

/// The isolate runtime as the machine host's script side.
struct RuntimeJs<'r> {
    runtime: &'r mut Runtime,
    claimed: &'r dyn Fn() -> bool,
}

impl machine::Js for RuntimeJs<'_> {
    fn queue_len(&mut self) -> usize {
        let mut scope = self.runtime.deno_runtime().handle_scope();
        js_queue_len(&mut scope)
    }

    fn call(&mut self, _hook: Option<&HeldCallback>, _args: &[Value]) -> Called {
        // V8 machine calls must carry their owning row.
        Called::Terminated
    }

    fn call_for(
        &mut self,
        handle: machine::Handle,
        hook: Option<&HeldCallback>,
        args: &[Value],
    ) -> Called {
        let called = {
            let scope = &mut self.runtime.deno_runtime().handle_scope();
            call_hook(scope, handle, hook, args, true)
        };
        // `drive` found the pass unhalted just before this call. A
        // terminate armed since then hit this call's own JS: its
        // microtasks run as the outermost call returns, and V8 consumes a
        // termination there without reporting it.
        match called {
            Called::Settled(_) | Called::Pending(_) if (self.claimed)() => Called::Terminated,
            called => called,
        }
    }

    fn poll(&mut self, pending: &Pending) -> Option<Reply> {
        let scope = &mut self.runtime.deno_runtime().handle_scope();
        poll_promise(scope, pending)
    }

    fn claimed(&mut self) -> bool {
        (self.claimed)()
    }

    fn game_messages_mark(&mut self) -> f64 {
        let scope = &mut self.runtime.deno_runtime().handle_scope();
        super::chatbox_v8::native_mark(scope)
    }
}

fn call_hook(
    scope: &mut v8::HandleScope,
    handle: machine::Handle,
    hook: Option<&HeldCallback>,
    args: &[Value],
    drain_microtasks: bool,
) -> Called {
    let Some(hook) = hook else {
        return Called::Settled(Reply::Threw(Thrown::new("undeclared machine callback")));
    };
    match super::lineage_v8::enter_hook(scope, handle, |scope| {
        call_hook_inner(scope, hook, args, drain_microtasks)
    }) {
        Ok(called) => called,
        Err(Throw::Value(exception)) => Called::Settled(Reply::Threw(thrown(scope, exception))),
        Err(Throw::Terminated) => Called::Terminated,
    }
}

fn call_hook_inner(
    scope: &mut v8::HandleScope,
    hook: &HeldCallback,
    args: &[Value],
    drain_microtasks: bool,
) -> Called {
    let callback = hook.open(scope);
    let mut argv = Vec::with_capacity(args.len());
    for arg in args {
        match serde_v8::to_v8(scope, arg) {
            Ok(value) => argv.push(value),
            Err(e) => {
                let failure = Thrown::new(format!("callback argument: {e}"));
                return Called::Settled(Reply::Threw(failure));
            }
        }
    }
    let value = match callback.call(scope, &argv) {
        Ok(value) => value,
        Err(Throw::Value(exception)) => {
            if checkpoint_terminated(scope, drain_microtasks) {
                return Called::Terminated;
            }
            return Called::Settled(Reply::Threw(thrown(scope, exception)));
        }
        // Rethrown; the host stops driving and no family sees it as a throw.
        Err(Throw::Terminated) => return Called::Terminated,
    };
    let Ok(promise) = v8::Local::<v8::Promise>::try_from(value) else {
        if checkpoint_terminated(scope, drain_microtasks) {
            return Called::Terminated;
        }
        return Called::Settled(settled_value(scope, value));
    };
    // The host observes the rejection; it is not an unhandled one.
    if let Some(noop) = v8::Function::new(
        scope,
        |_: &mut v8::HandleScope, _: v8::FunctionCallbackArguments, _: v8::ReturnValue| {},
    ) {
        promise.catch(scope, noop);
    }
    // An `await` on something already settled resumes here, as it
    // would in the frozen driver's own microtask flush. A kick skips
    // the isolate drain and polls this promise instead (M-187).
    if checkpoint_terminated(scope, drain_microtasks) {
        return Called::Terminated;
    }
    Called::Pending(v8::Global::new(scope, promise))
}

/// Optionally run the callback's microtasks, then report termination.
///
/// A tick step is the outermost JS, so it drains: an already-settled
/// `await` answers in this call (N1(a)), and a continuation's terminate
/// is visible. A kick is nested in the caller's turn; draining would run
/// unrelated queued microtasks before the caller continues (M-187). The
/// host still polls this callback's promise, so an already-fulfilled
/// return settles without touching the rest of the queue. Inside a kick a
/// continuation runs later, in the caller's microtasks, so its
/// termination is not seen here. At the outermost level V8 consumes it;
/// [`RuntimeJs`] reports that case from the host's halt flag.
fn checkpoint_terminated(scope: &mut v8::HandleScope, drain_microtasks: bool) -> bool {
    if drain_microtasks {
        scope.perform_microtask_checkpoint();
    }
    scope.is_execution_terminating()
}

fn poll_promise(scope: &mut v8::HandleScope, pending: &Pending) -> Option<Reply> {
    let promise = v8::Local::new(scope, pending);
    match promise.state() {
        v8::PromiseState::Pending => None,
        v8::PromiseState::Fulfilled => {
            let value = promise.result(scope);
            Some(settled_value(scope, value))
        }
        v8::PromiseState::Rejected => {
            let reason = promise.result(scope);
            Some(Reply::Threw(thrown(scope, reason)))
        }
    }
}

fn settled_value(scope: &mut v8::HandleScope, value: v8::Local<v8::Value>) -> Reply {
    match serde_v8::from_v8::<Value>(scope, value) {
        Ok(value) => Reply::Value(value),
        Err(e) => Reply::Threw(Thrown::new(format!("callback result: {e}"))),
    }
}

/// The thrown value itself, with its message (`new Error('x')` → `x`,
/// else V8's text).
fn thrown(scope: &mut v8::HandleScope, value: v8::Local<v8::Value>) -> Thrown {
    let error = JsError::from_v8_exception(scope, value);
    let message = error.message.unwrap_or(error.exception_message);
    Thrown::js(message, v8::Global::new(scope, value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustyscript::RuntimeOptions;
    use script::api_combat::{CombatEnd, CombatSummary, ReportEnd};
    use script::combat::PvpSource;

    /// V8 used heap after a full collection.
    fn collected_used_heap(runtime: &mut Runtime) -> usize {
        let isolate = runtime.deno_runtime().v8_isolate();
        isolate.low_memory_notification();
        isolate.get_heap_statistics().used_heap_size()
    }

    fn summary(index: u64) -> CombatSummary {
        CombatSummary {
            end: ReportEnd::Killed,
            reason: None,
            npc_type: 81,
            ticks: 40,
            swings: 9,
            casts: 0,
            damage_taken: 3,
            food: 1,
            prayer_doses: 0,
            boost_doses: 0,
            antifire_doses: 0,
            protect_switches: 0,
            pvp_source: match index % 4 {
                0 => None,
                1 => Some(PvpSource::Request),
                2 => Some(PvpSource::Global),
                _ => Some(PvpSource::Both),
            },
        }
    }

    /// PVP-DESIGN-1026 §9.6: 1 000 `api.combat.fight` summaries, built by the
    /// host's own `outcome_envelope` from the same JSON the host publishes and
    /// retained in one array, grow the collected V8 used heap by no more than
    /// the bound, and every one carries all thirteen report fields (`pvpSource`
    /// is present, as `null`, when no player fight was started).
    #[test]
    fn a_thousand_combat_summaries_bound_the_collected_v8_heap() {
        const SUMMARIES: u64 = 1_000;
        // Measured: 359 424 bytes (about 351 KiB; each summary is three
        // fast-property objects built from cached templates), down from
        // 1 341 024 bytes when the shared `serde_v8` path built them in
        // dictionary mode; the bound is 1.5x of the measure.
        const HEAP_GROWTH_LIMIT: usize = 512 * 1024;
        crate::runtime::platform::bootstrap_test_v8_platform();
        let mut runtime = Runtime::new(RuntimeOptions::default()).unwrap();
        callback_v8::install_ops(&mut runtime).unwrap();
        install(&mut runtime).unwrap();
        runtime.eval::<()>("globalThis.__kept = null;").unwrap();
        let baseline = collected_used_heap(&mut runtime);
        {
            let context = runtime.deno_runtime().main_context();
            let mut scope = runtime.deno_runtime().handle_scope();
            let global = context.open(&mut scope).global(&mut scope);
            let kept = v8::Array::new(&mut scope, 0);
            for index in 0..SUMMARIES {
                let end = CombatEnd::Fought {
                    token: index + 1,
                    report: summary(index),
                };
                let envelope = outcome_envelope(&mut scope, Outcome::Done(Value::from(end)))
                    .unwrap_or_else(|_| panic!("summary {index} materializes"));
                kept.set_index(&mut scope, index as u32, envelope);
            }
            let key = callback_v8::string(&mut scope, "__kept");
            global.set(&mut scope, key, kept.into());
        }
        let after = collected_used_heap(&mut runtime);
        let growth = after.saturating_sub(baseline);
        println!(
            "combat summary heap: summaries={SUMMARIES} baseline={baseline} after={after} growth={growth} limit={HEAP_GROWTH_LIMIT}"
        );
        assert!(
            growth <= HEAP_GROWTH_LIMIT,
            "{SUMMARIES} retained summaries grew the collected V8 heap by {growth} bytes (limit {HEAP_GROWTH_LIMIT})"
        );
        let all_keys = runtime
            .eval::<bool>(
                r#"(() => {
                    const want = ['end','reason','npcType','ticks','swings','casts','damageTaken','food',
                        'prayerDoses','boostDoses','antifireDoses','protectSwitches','pvpSource'].join();
                    return __kept.length === 1000 && __kept.every((r, i) =>
                        r.kind === 'done' && r.value.report && Object.keys(r.value.report).join() === want
                        && r.value.report.pvpSource === [null, 'request', 'global', 'both'][i % 4]);
                })()"#,
            )
            .unwrap();
        assert!(
            all_keys,
            "every summary has all thirteen report fields in order"
        );
    }

    #[test]
    fn generic_value_rows_bound_collected_v8_heap_growth() {
        const ROWS: usize = 4_096;
        // Allow 1 KiB per retained row, matching the established entity-row bound.
        const MAX_HEAP_GROWTH: usize = 4 * 1024 * 1024;
        crate::runtime::platform::bootstrap_test_v8_platform();
        let mut runtime = Runtime::new(RuntimeOptions::default()).unwrap();
        callback_v8::install_ops(&mut runtime).unwrap();
        install(&mut runtime).unwrap();

        let warm = serde_json::json!({
            "id": 0,
            "name": "Coins",
            "actions": ["Take"],
            "tile": { "x": 2737, "z": 3440 },
        });
        {
            let mut scope = runtime.deno_runtime().handle_scope();
            value_to_v8(&mut scope, &warm, 0).expect("warm row materializes");
        }
        let baseline = collected_used_heap(&mut runtime);
        {
            let context = runtime.deno_runtime().main_context();
            let mut scope = runtime.deno_runtime().handle_scope();
            let global = context.open(&mut scope).global(&mut scope);
            let kept = v8::Array::new(&mut scope, ROWS as i32);
            for index in 0..ROWS {
                let row = serde_json::json!({
                    "id": index,
                    "name": "Coins",
                    "actions": ["Take"],
                    "tile": { "x": 2737, "z": 3440 },
                });
                let value = value_to_v8(&mut scope, &row, 0)
                    .unwrap_or_else(|error| panic!("row {index} materializes: {error}"));
                assert!(
                    kept.set_index(&mut scope, index as u32, value),
                    "V8 retains row {index}"
                );
            }
            let key = callback_v8::string(&mut scope, "__value_rows");
            global.set(&mut scope, key, kept.into());
        }
        let after = collected_used_heap(&mut runtime);
        let growth = after.saturating_sub(baseline);
        println!(
            "generic machine values collected V8 heap: rows={ROWS} baseline={baseline} \
             after={after} growth={growth} limit={MAX_HEAP_GROWTH}"
        );
        assert!(
            growth <= MAX_HEAP_GROWTH,
            "{ROWS} retained generic values grew the collected V8 used heap by {growth} \
             bytes; limit {MAX_HEAP_GROWTH} (baseline {baseline}, after {after})"
        );
        assert_eq!(
            runtime.eval::<usize>("__value_rows.length").unwrap(),
            ROWS,
            "all materialized rows remain reachable"
        );
    }

    const CAPACITY_TEST_NAME: &str =
        "materialize::machine_v8::tests::shaped_values_equal_the_serde_values_and_the_cache_stays_bounded";
    const CAPACITY_CHILD_ENV: &str = "SCRIPT_MACHINE_VALUE_SHAPE_TEST_CHILD";

    /// `value_to_v8` hands the script the same data `serde_v8::to_v8` would
    /// (scalars, nested arrays and objects, key order, numbers outside `i32`,
    /// empty containers, objects beyond the shape limits and past the shape
    /// cache), keeps the cache bounded, and, with the cache full, still builds
    /// every object with fast properties and `Object.prototype` (AGENTS.md: a
    /// script-facing object is never in dictionary mode). `%HasFastProperties`
    /// needs `--allow-natives-syntax` before V8 starts, so the body runs in a
    /// child process (as `bank_result_shapes_cover_callbacks_and_bound_v8_heap`).
    #[test]
    fn shaped_values_equal_the_serde_values_and_the_cache_stays_bounded() {
        if std::env::var_os(CAPACITY_CHILD_ENV).is_none() {
            let output = std::process::Command::new(
                std::env::current_exe().expect("unit-test executable path"),
            )
            .args(["--exact", CAPACITY_TEST_NAME, "--nocapture"])
            .env(CAPACITY_CHILD_ENV, "1")
            .output()
            .expect("run isolated V8-native-syntax test");
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(
                output.status.success(),
                "isolated V8 shape test failed\nstdout: {stdout}\nstderr: {stderr}"
            );
            assert!(
                stdout.contains(&format!("test {CAPACITY_TEST_NAME} ... ok"))
                    && stdout.contains("1 passed"),
                "isolated test did not run exactly once\nstdout: {stdout}\nstderr: {stderr}"
            );
            return;
        }
        v8::V8::set_flags_from_string("--allow-natives-syntax");
        crate::runtime::platform::bootstrap_test_v8_platform();
        let mut runtime = Runtime::new(RuntimeOptions::default()).unwrap();
        callback_v8::install_ops(&mut runtime).unwrap();
        install(&mut runtime).unwrap();
        let wide: serde_json::Map<String, Value> = (0..=MAX_SHAPE_KEYS)
            .map(|index| (format!("k{index}"), Value::from(index)))
            .collect();
        let mut deep = Value::from(1);
        for _ in 0..MAX_VALUE_DEPTH + 2 {
            deep = serde_json::json!({ "inner": deep });
        }
        let mut values = vec![
            Value::Null,
            Value::from(true),
            Value::from(-7),
            Value::from(0.25),
            Value::from(4_294_967_296_u64),
            Value::from("héllo ✓"),
            serde_json::json!([]),
            serde_json::json!({}),
            serde_json::json!({ "z": 1, "a": [1, "two", null, { "b": false }], "m": { "x": 1.5 } }),
            serde_json::json!({ "z": 2, "a": [], "m": { "x": 2.5 } }),
            Value::Object(wide),
            deep,
        ];
        // More distinct shapes than the cache holds.
        for index in 0..MAX_VALUE_SHAPES + 8 {
            values.push(serde_json::json!({ format!("shape{index}"): index }));
        }
        {
            let mut scope = runtime.deno_runtime().handle_scope();
            for value in &values {
                let shaped = value_to_v8(&mut scope, value, 0).expect("shaped value");
                let serde = serde_v8::to_v8(&mut scope, value).expect("serde value");
                let shaped: Value = serde_v8::from_v8(&mut scope, shaped).unwrap();
                let serde: Value = serde_v8::from_v8(&mut scope, serde).unwrap();
                assert_eq!(shaped, serde, "{value}");
                // Same key order as the input object, which `from_v8` keeps.
                assert_eq!(shaped.to_string(), serde.to_string(), "{value}");
            }
        }
        let cached = runtime
            .deno_runtime()
            .v8_isolate()
            .get_slot::<Rc<RefCell<ValueShapes>>>()
            .expect("installed")
            .borrow()
            .shapes
            .len();
        assert_eq!(cached, MAX_VALUE_SHAPES, "the cache is full and stays full");

        // With the cache full: a summary (three objects), a wide object and
        // a shape never seen before.
        {
            let context = runtime.deno_runtime().main_context();
            let mut scope = runtime.deno_runtime().handle_scope();
            let global = context.open(&mut scope).global(&mut scope);
            let end = CombatEnd::Fought {
                token: 1,
                report: summary(0),
            };
            let envelope = outcome_envelope(&mut scope, Outcome::Done(Value::from(end)))
                .unwrap_or_else(|_| panic!("summary materializes"));
            let wide: serde_json::Map<String, Value> = (0..=MAX_SHAPE_KEYS)
                .map(|index| (format!("w{index}"), Value::from(index)))
                .collect();
            let wide = value_to_v8(&mut scope, &Value::Object(wide), 0).unwrap();
            let unseen = value_to_v8(&mut scope, &serde_json::json!({ "unseen": 1 }), 0).unwrap();
            for (name, value) in [
                ("__envelope", envelope),
                ("__wide", wide),
                ("__unseen", unseen),
            ] {
                let key = callback_v8::string(&mut scope, name);
                global.set(&mut scope, key, value);
            }
        }
        let facts = runtime
            .eval::<Value>(
                r#"(() => {
                    const fast = new Function('o', 'return %HasFastProperties(o)');
                    const check = (o) => ({
                        fast: fast(o),
                        prototype: Object.getPrototypeOf(o) === Object.prototype,
                    });
                    return {
                        envelope: check(__envelope),
                        value: check(__envelope.value),
                        report: check(__envelope.value.report),
                        wide: check(__wide),
                        unseen: check(__unseen),
                    };
                })()"#,
            )
            .unwrap();
        let good = serde_json::json!({ "fast": true, "prototype": true });
        for name in ["envelope", "value", "report", "wide", "unseen"] {
            assert_eq!(
                facts[name], good,
                "{name} must keep fast properties and Object.prototype with the shape cache full"
            );
        }
    }
}
