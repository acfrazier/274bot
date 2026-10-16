//! Per-isolate CPED lineage carrier and Rust-to-JavaScript entry guards.

use crate::load::callback_v8::{self, Throw};
use crate::machine::{CapturedLineage, Handle};
use rustyscript::deno_core::v8;
use rustyscript::{Error, Runtime};
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::cell::{Cell, RefCell};
use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};
use std::rc::Rc;
use std::time::Duration;
const MAX_CAPTURED_INTERACT_ROWS: usize =
    api::script_contract::MAX_INTERACT_REQS + api::script_contract::MAX_RELEASES;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Lineage {
    Root,
    Token(Handle),
    Foreign,
}

struct CarrierState {
    key: v8::Global<v8::Symbol>,
    hooks: RefCell<Vec<Handle>>,
    captured_lineages: RefCell<Vec<CapturedLineage>>,
    interact_gate_enabled: bool,
    interact_overflow_dropped: Cell<usize>,
    active_entries: Cell<usize>,
    foreign_reported: Cell<bool>,
    foreign_pending: Cell<bool>,
}

struct EntryGuard<'a>(&'a CarrierState);

impl<'a> EntryGuard<'a> {
    fn new(state: &'a CarrierState) -> Self {
        state.active_entries.set(state.active_entries.get() + 1);
        Self(state)
    }
}

impl Drop for EntryGuard<'_> {
    fn drop(&mut self) {
        self.0.active_entries.set(self.0.active_entries.get() - 1);
    }
}

struct HookGuard<'a> {
    state: &'a CarrierState,
    handle: Handle,
}

impl<'a> HookGuard<'a> {
    fn new(state: &'a CarrierState, handle: Handle) -> Self {
        state.hooks.borrow_mut().push(handle);
        Self { state, handle }
    }
}

impl Drop for HookGuard<'_> {
    fn drop(&mut self) {
        let popped = self.state.hooks.borrow_mut().pop();
        debug_assert_eq!(popped, Some(self.handle));
    }
}

/// Install the fresh symbol and carrier state on this isolate.
pub(crate) fn install(runtime: &mut Runtime, interact_gate_enabled: bool) -> Result<(), String> {
    let capacity = if interact_gate_enabled {
        MAX_CAPTURED_INTERACT_ROWS
    } else {
        api::script_contract::MAX_INTERACT_REQS
    };
    let state = {
        let mut scope = runtime.deno_runtime().handle_scope();
        let key = v8::Symbol::new(&mut scope, None);
        Rc::new(CarrierState {
            key: v8::Global::new(&mut scope, key),
            hooks: RefCell::new(Vec::new()),
            captured_lineages: RefCell::new(Vec::with_capacity(capacity)),
            interact_gate_enabled,
            interact_overflow_dropped: Cell::new(0),
            active_entries: Cell::new(0),
            foreign_reported: Cell::new(false),
            foreign_pending: Cell::new(false),
        })
    };
    if runtime.deno_runtime().v8_isolate().set_slot(state) {
        Ok(())
    } else {
        Err("lineage carrier already installed".into())
    }
}

fn state_for_scope(scope: &v8::HandleScope) -> Option<Rc<CarrierState>> {
    scope.get_slot::<Rc<CarrierState>>().cloned()
}

fn state_for_runtime(runtime: &mut Runtime) -> Option<Rc<CarrierState>> {
    runtime
        .deno_runtime()
        .v8_isolate()
        .get_slot::<Rc<CarrierState>>()
        .cloned()
}

fn mark_foreign(state: &CarrierState) {
    if !state.foreign_reported.replace(true) {
        state.foreign_pending.set(true);
    }
}

/// Read the current slot without applying the synchronous-hook fallback.
pub(crate) fn read_lineage(scope: &mut v8::HandleScope) -> Lineage {
    let Some(state) = state_for_scope(scope) else {
        return Lineage::Foreign;
    };
    let current = scope.get_continuation_preserved_embedder_data();
    if current.is_undefined() {
        return Lineage::Root;
    }
    let Ok(mapping) = v8::Local::<v8::Object>::try_from(current) else {
        return Lineage::Foreign;
    };
    let key = v8::Local::new(scope, &state.key);
    let parsed = callback_v8::catching(scope, |scope| {
        if mapping.has_own_property(scope, key.into())? {
            let value = mapping.get(scope, key.into())?;
            let token = v8::Local::<v8::BigInt>::try_from(value).ok()?;
            let (handle, lossless) = token.u64_value();
            return Some(if lossless {
                Lineage::Token(handle)
            } else {
                Lineage::Foreign
            });
        }
        Some(Lineage::Foreign)
    });
    match parsed {
        Ok(lineage) => lineage,
        Err(_) => Lineage::Foreign,
    }
}

/// Resolve a foreign context to the innermost synchronous hook, or to root.
/// The first foreign value also queues one diagnostic for the isolate run.
pub(crate) fn resolve_lineage(scope: &mut v8::HandleScope) -> Lineage {
    match read_lineage(scope) {
        Lineage::Foreign => {
            let Some(state) = state_for_scope(scope) else {
                return Lineage::Foreign;
            };
            mark_foreign(&state);
            let lineage = state
                .hooks
                .borrow()
                .last()
                .copied()
                .map_or(Lineage::Root, Lineage::Token);
            lineage
        }
        lineage => lineage,
    }
}

/// Return the pending carrier diagnostic once, for the isolate-thread logger.
pub(crate) fn take_foreign_diagnostic(runtime: &mut Runtime) -> bool {
    state_for_runtime(runtime).is_some_and(|state| state.foreign_pending.replace(false))
}

/// Assert that a native callback runs inside a root or hook guard.
pub(crate) fn assert_guarded(scope: &v8::HandleScope) {
    debug_assert!(state_for_scope(scope).is_some_and(|state| state.active_entries.get() != 0));
}

fn capture_count<'s>(
    scope: &mut v8::HandleScope<'s>,
    value: v8::Local<'s, v8::Value>,
    max: usize,
    invalid: &'static str,
) -> Result<usize, Throw<'s>> {
    let Ok(number) = v8::Local::<v8::Number>::try_from(value) else {
        return Err(callback_v8::type_error(scope, invalid));
    };
    let count = number.value();
    if !count.is_finite() || count.fract() != 0.0 || count < 0.0 || count > max as f64 {
        return Err(callback_v8::type_error(scope, invalid));
    }
    Ok(count as usize)
}

fn capture_interact_lineage<'s>(
    scope: &mut v8::HandleScope<'s>,
    count: usize,
    dropped: usize,
) -> Result<(), Throw<'s>> {
    let Some(state) = state_for_scope(scope) else {
        return Err(callback_v8::type_error(
            scope,
            "lineage carrier is not installed",
        ));
    };
    if state.active_entries.get() == 0 {
        return Err(callback_v8::type_error(
            scope,
            "interact push outside guarded JS entry",
        ));
    }
    let capacity = if state.interact_gate_enabled {
        MAX_CAPTURED_INTERACT_ROWS
    } else {
        api::script_contract::MAX_INTERACT_REQS
    };
    let mut captured = state.captured_lineages.borrow_mut();
    let Some(new_len) = captured.len().checked_add(count) else {
        return Err(callback_v8::type_error(
            scope,
            "interact lineage capacity exceeded",
        ));
    };
    if new_len > capacity {
        return Err(callback_v8::type_error(
            scope,
            "interact lineage capacity exceeded",
        ));
    }
    if count != 0 {
        let lineage = match resolve_lineage(scope) {
            Lineage::Root => CapturedLineage::default(),
            Lineage::Token(handle) => crate::machine::capture_push(Some(handle)),
            Lineage::Foreign => {
                return Err(callback_v8::type_error(
                    scope,
                    "unresolved foreign interact lineage",
                ));
            }
        };
        if lineage.lineage.is_some() && lineage.lineage == lineage.mirror_owner {
            return Err(callback_v8::type_error(
                scope,
                "interact lineage collides with mirror owner",
            ));
        }
        captured.extend(std::iter::repeat(lineage).take(count));
    }
    drop(captured);
    state.interact_overflow_dropped.set(
        state
            .interact_overflow_dropped
            .get()
            .saturating_add(dropped),
    );
    Ok(())
}

fn interact_gate_config<'s>(
    scope: &mut v8::HandleScope<'s>,
) -> Result<v8::Local<'s, v8::Value>, Throw<'s>> {
    let Some(state) = state_for_scope(scope) else {
        return Err(callback_v8::type_error(
            scope,
            "lineage carrier is not installed",
        ));
    };
    let config = v8::Array::new(scope, 2);
    let gameplay = if state.interact_gate_enabled {
        api::script_contract::MAX_INTERACT_REQS
    } else {
        0
    };
    let gameplay = v8::Number::new(scope, gameplay as f64);
    let walk_releases = v8::Number::new(scope, api::script_contract::MAX_WALK_RELEASES as f64);
    if !config.set_index(scope, 0, gameplay.into()).unwrap_or(false)
        || !config
            .set_index(scope, 1, walk_releases.into())
            .unwrap_or(false)
    {
        return Err(callback_v8::type_error(
            scope,
            "interact gate configuration failed",
        ));
    }
    Ok(config.into())
}

fn capture_interact_lineage_callback<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = if args.length() == 0 {
        interact_gate_config(scope)
    } else {
        let Some(state) = state_for_scope(scope) else {
            let error = callback_v8::type_error(scope, "lineage carrier is not installed");
            callback_v8::finish(scope, rv, Err(error));
            return;
        };
        let capacity = if state.interact_gate_enabled {
            MAX_CAPTURED_INTERACT_ROWS
        } else {
            api::script_contract::MAX_INTERACT_REQS
        };
        capture_count(
            scope,
            args.get(0),
            capacity,
            "invalid interact lineage count",
        )
        .and_then(|count| {
            capture_count(
                scope,
                args.get(1),
                u32::MAX as usize,
                "invalid interact overflow count",
            )
            .and_then(|dropped| {
                capture_interact_lineage(scope, count, dropped)
                    .map(|()| v8::undefined(scope).into())
            })
        })
    };
    callback_v8::finish(scope, rv, result);
}

fn discard_interact_lineage_callback<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = (|| {
        let Some(state) = state_for_scope(scope) else {
            return Err(callback_v8::type_error(
                scope,
                "lineage carrier is not installed",
            ));
        };
        if state.active_entries.get() == 0 {
            return Err(callback_v8::type_error(
                scope,
                "interact rollback outside guarded JS entry",
            ));
        }
        let capacity = if state.interact_gate_enabled {
            MAX_CAPTURED_INTERACT_ROWS
        } else {
            api::script_contract::MAX_INTERACT_REQS
        };
        let count = capture_count(
            scope,
            args.get(0),
            capacity,
            "invalid interact lineage count",
        )?;
        let dropped = capture_count(
            scope,
            args.get(1),
            u32::MAX as usize,
            "invalid interact overflow count",
        )?;
        let mut captured = state.captured_lineages.borrow_mut();
        if count > captured.len() {
            return Err(callback_v8::type_error(
                scope,
                "interact lineage rollback mismatch",
            ));
        }
        let kept = captured.len() - count;
        captured.truncate(kept);
        drop(captured);
        let pending_dropped = state.interact_overflow_dropped.get();
        if dropped > pending_dropped {
            return Err(callback_v8::type_error(
                scope,
                "interact overflow rollback mismatch",
            ));
        }
        state
            .interact_overflow_dropped
            .set(pending_dropped - dropped);
        Ok(v8::undefined(scope).into())
    })();
    callback_v8::finish(scope, rv, result);
}

/// Install the private callbacks used by the host's append-only interact queue.
pub(crate) fn install_interact_capture(runtime: &mut Runtime) -> Result<(), String> {
    callback_v8::install(
        runtime,
        "__rs2b0t_capture_interact_lineage",
        capture_interact_lineage_callback,
    )?;
    callback_v8::install(
        runtime,
        "__rs2b0t_discard_interact_lineage",
        discard_interact_lineage_callback,
    )
}

/// Move the bounded push-time metadata out with the corresponding batch bytes.
pub(crate) fn take_interact_lineages(
    runtime: &mut Runtime,
) -> Result<Vec<CapturedLineage>, String> {
    let state =
        state_for_runtime(runtime).ok_or_else(|| "lineage carrier is not installed".to_string())?;
    let capacity = if state.interact_gate_enabled {
        MAX_CAPTURED_INTERACT_ROWS
    } else {
        api::script_contract::MAX_INTERACT_REQS
    };
    let captured = std::mem::replace(
        &mut *state.captured_lineages.borrow_mut(),
        Vec::with_capacity(capacity),
    );
    Ok(captured)
}

/// Take the number of rows the bounded owner producer dropped before capture.
pub(crate) fn take_interact_overflow(runtime: &mut Runtime) -> Result<usize, String> {
    let state =
        state_for_runtime(runtime).ok_or_else(|| "lineage carrier is not installed".to_string())?;
    Ok(state.interact_overflow_dropped.replace(0))
}

/// Discard metadata when its JS interact batch is intentionally discarded.
pub(crate) fn clear_interact_lineages(runtime: &mut Runtime) {
    if let Some(state) = state_for_runtime(runtime) {
        state.captured_lineages.borrow_mut().clear();
        state.interact_overflow_dropped.set(0);
    }
}

fn copy_context<'s>(
    scope: &mut v8::HandleScope<'s>,
    previous: v8::Local<'s, v8::Value>,
    key: v8::Local<'s, v8::Symbol>,
    handle: Handle,
) -> Option<v8::Local<'s, v8::Value>> {
    let mapping = v8::Object::new(scope);
    let null = v8::null(scope);
    if mapping.set_prototype(scope, null.into())? == false {
        return None;
    }
    if let Ok(previous) = v8::Local::<v8::Object>::try_from(previous) {
        let names = previous.get_own_property_names(
            scope,
            v8::GetPropertyNamesArgs {
                mode: v8::KeyCollectionMode::OwnOnly,
                property_filter: v8::PropertyFilter::ONLY_ENUMERABLE,
                index_filter: v8::IndexFilter::IncludeIndices,
                key_conversion: v8::KeyConversionMode::KeepNumbers,
            },
        )?;
        for index in 0..names.length() {
            let name = names.get_index(scope, index)?;
            let value = previous.get(scope, name)?;
            if mapping.set(scope, name, value)? == false {
                return None;
            }
        }
    }
    let token = v8::BigInt::new_from_u64(scope, handle);
    if mapping.set(scope, key.into(), token.into())? == false {
        return None;
    }
    Some(mapping.into())
}

/// Run one held machine hook with its row token in the composed CPED mapping.
pub(crate) fn enter_hook<'s, R>(
    scope: &mut v8::HandleScope<'s>,
    handle: Handle,
    call: impl FnOnce(&mut v8::HandleScope<'s>) -> R,
) -> Result<R, Throw<'s>> {
    let Some(state) = state_for_scope(scope) else {
        return Err(Throw::Terminated);
    };
    let previous = scope.get_continuation_preserved_embedder_data();
    let _entry = EntryGuard::new(&state);
    let _hook = HookGuard::new(&state, handle);
    let key = v8::Local::new(scope, &state.key);
    let mapping = callback_v8::catching(scope, |scope| copy_context(scope, previous, key, handle))?;
    scope.set_continuation_preserved_embedder_data(mapping);

    let result = catch_unwind(AssertUnwindSafe(|| call(scope)));
    scope.set_continuation_preserved_embedder_data(previous);
    match result {
        Ok(value) => Ok(value),
        Err(panic) => resume_unwind(panic),
    }
}

fn clear_context(runtime: &mut Runtime) {
    let mut scope = runtime.deno_runtime().handle_scope();
    let undefined = v8::undefined(&mut scope);
    scope.set_continuation_preserved_embedder_data(undefined.into());
}

/// Run one Rust-to-JavaScript entry with an undefined ambient CPED value.
pub(crate) fn enter_root<R>(runtime: &mut Runtime, call: impl FnOnce(&mut Runtime) -> R) -> R {
    let state = state_for_runtime(runtime);
    let entry = state.as_ref().map(|state| EntryGuard::new(state.as_ref()));
    clear_context(runtime);
    let result = catch_unwind(AssertUnwindSafe(|| call(runtime)));
    clear_context(runtime);
    drop(entry);
    match result {
        Ok(value) => value,
        Err(panic) => resume_unwind(panic),
    }
}

pub(crate) fn call_function_immediate<T: DeserializeOwned>(
    runtime: &mut Runtime,
    module_context: Option<&rustyscript::ModuleHandle>,
    name: &str,
    args: &impl Serialize,
) -> Result<T, Error> {
    enter_root(runtime, |runtime| {
        runtime.call_function_immediate(module_context, name, args)
    })
}

pub(crate) fn eval<T: DeserializeOwned>(
    runtime: &mut Runtime,
    expression: impl ToString,
) -> Result<T, Error> {
    enter_root(runtime, |runtime| runtime.eval(expression))
}

pub(crate) fn block_on_event_loop(
    runtime: &mut Runtime,
    options: rustyscript::deno_core::PollEventLoopOptions,
    timeout: Option<Duration>,
) -> Result<(), Error> {
    enter_root(runtime, |runtime| {
        runtime.block_on_event_loop(options, timeout)
    })
}
pub(crate) fn load_modules(
    runtime: &mut Runtime,
    main: &rustyscript::Module,
    side: Vec<&rustyscript::Module>,
) -> Result<(), Error> {
    enter_root(runtime, |runtime| {
        runtime.load_modules(main, side).map(|_| ())
    })
}
