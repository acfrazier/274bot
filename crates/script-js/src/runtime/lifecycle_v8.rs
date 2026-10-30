//! Native runtime registries and run-context handles exposed to compat shims.
//!
//! This is the isolate-local owner for caller-provided runtime descriptors.
//! The static catalog parser remains an input adapter; this registry retains
//! the exact JavaScript metadata object and its original `create` callback.

use crate::callback_v8::{self as cb, JsResult};
use crate::lineage_v8;
use api::local_state::{DomainEdit, LocalStateDomain, RuntimeSettingsEdit, SubmitReceipt};
use rustyscript::deno_core::v8;
use rustyscript::Runtime;
use script::local_state::SlotState;
use script::surface::{RuntimeRunState, ScriptRuntimeContext};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

const REGISTRY_DISPATCHER: &str = "__rs2b0t_script_registry";
const SETTINGS_DISPATCHER: &str = "__rs2b0t_settings_store";
const RUNTIME_DISPATCHER: &str = "__rs2b0t_script_runtime";

struct RegistryEntry {
    metadata: v8::Global<v8::Object>,
    origin: Option<String>,
}

#[derive(Default)]
struct RegistryState {
    entries: RefCell<Vec<RegistryEntry>>,
    by_name: RefCell<HashMap<String, usize>>,
    by_origin: RefCell<HashMap<String, usize>>,
    runtime_context: RefCell<Option<Arc<ScriptRuntimeContext>>>,
}
struct SettingsListener {
    callback: Option<v8::Global<v8::Function>>,
    order: u64,
}

struct SettingsState {
    local_state: Option<Arc<SlotState>>,
    listeners: RefCell<Vec<SettingsListener>>,
    next_order: Cell<u64>,
}

/// Install native registry and local settings dispatchers for one isolate.
pub(crate) fn install(
    runtime: &mut Runtime,
    local_state: Option<Arc<SlotState>>,
) -> Result<(), String> {
    let registry = Rc::new(RegistryState::default());
    if !runtime.deno_runtime().v8_isolate().set_slot(registry) {
        return Err("runtime registry state already installed".into());
    }
    cb::install(runtime, REGISTRY_DISPATCHER, registry_callback)?;

    let settings = Rc::new(SettingsState {
        local_state,
        next_order: Cell::new(0),
        listeners: RefCell::new(Vec::new()),
    });
    if !runtime.deno_runtime().v8_isolate().set_slot(settings) {
        return Err("runtime settings state already installed".into());
    }
    cb::install(runtime, SETTINGS_DISPATCHER, settings_callback)?;
    cb::install(runtime, RUNTIME_DISPATCHER, runtime_callback)
}

/// Bind the slot-owned context before the next isolate command can read it.
pub(crate) fn bind_runtime_context(
    runtime: &mut Runtime,
    context: Arc<ScriptRuntimeContext>,
) -> Result<(), String> {
    let scope = runtime.deno_runtime().handle_scope();
    let state = scope
        .get_slot::<Rc<RegistryState>>()
        .cloned()
        .ok_or_else(|| "runtime registry state is not installed".to_string())?;
    *state.runtime_context.borrow_mut() = Some(context);
    Ok(())
}
fn state_for_scope(scope: &v8::HandleScope) -> Option<Rc<RegistryState>> {
    scope.get_slot::<Rc<RegistryState>>().cloned()
}

fn registry_callback<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = dispatch(scope, args);
    cb::finish(scope, rv, result);
}

fn dispatch<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let op = args.get(0);
    if !op.is_string() {
        return Err(cb::type_error(
            scope,
            "ScriptRegistry operation must be a string",
        ));
    }
    let op = op.to_rust_string_lossy(scope);
    let state = state_for_scope(scope)
        .ok_or_else(|| cb::type_error(scope, "ScriptRegistry native state is not installed"))?;
    match op.as_str() {
        "register" => {
            let metadata = args.get(1);
            let metadata = v8::Local::<v8::Object>::try_from(metadata)
                .map_err(|_| cb::type_error(scope, "ScriptRegistry metadata must be an object"))?;
            register_metadata(scope, &state, metadata, None)?;
            Ok(v8::undefined(scope).into())
        }
        "registerScript" => register_script(scope, &state, args),
        "list" => {
            let entries = state.entries.borrow();
            let length = i32::try_from(entries.len())
                .map_err(|_| cb::type_error(scope, "ScriptRegistry has too many entries"))?;
            let values = v8::Array::new(scope, length);
            for (index, entry) in entries.iter().enumerate() {
                let value = v8::Local::new(scope, &entry.metadata);
                if !values
                    .set_index(scope, index as u32, value.into())
                    .unwrap_or(false)
                {
                    return Err(cb::type_error(
                        scope,
                        "ScriptRegistry list could not retain metadata",
                    ));
                }
            }
            Ok(values.into())
        }
        "get" => {
            let name = args.get(1);
            if !name.is_string() {
                return Err(cb::type_error(
                    scope,
                    "ScriptRegistry name must be a string",
                ));
            }
            let name = name.to_rust_string_lossy(scope);
            let by_name = state.by_name.borrow();
            let index = by_name
                .get(&name)
                .or_else(|| {
                    (name == "NatureCrafter")
                        .then(|| by_name.get("JiveRC"))
                        .flatten()
                })
                .copied();
            let Some(index) = index else {
                return Ok(v8::undefined(scope).into());
            };
            let entries = state.entries.borrow();
            let Some(entry) = entries.get(index) else {
                return Err(cb::type_error(
                    scope,
                    "ScriptRegistry index is inconsistent",
                ));
            };
            let metadata = v8::Local::new(scope, &entry.metadata);
            Ok(metadata.into())
        }
        _ => Err(cb::type_error(scope, "unknown ScriptRegistry operation")),
    }
}
fn register_metadata<'s>(
    scope: &mut v8::HandleScope<'s>,
    state: &RegistryState,
    metadata: v8::Local<'s, v8::Object>,
    origin: Option<String>,
) -> JsResult<'s, ()> {
    let name = cb::get(scope, metadata.into(), "name")?;
    if !name.is_string() {
        return Err(cb::type_error(
            scope,
            "ScriptRegistry metadata name must be a string",
        ));
    }
    let name = name.to_rust_string_lossy(scope);
    let entry = RegistryEntry {
        metadata: v8::Global::new(scope, metadata),
        origin: origin.clone(),
    };
    let existing_index = state.by_name.borrow().get(&name).copied();
    let index = if let Some(index) = existing_index {
        let old_origin = {
            let mut entries = state.entries.borrow_mut();
            let Some(current) = entries.get_mut(index) else {
                return Err(cb::type_error(
                    scope,
                    "ScriptRegistry index is inconsistent",
                ));
            };
            let old_origin = current.origin.take();
            *current = entry;
            old_origin
        };
        if let Some(old_origin) = old_origin {
            let mut by_origin = state.by_origin.borrow_mut();
            if by_origin.get(&old_origin) == Some(&index) {
                by_origin.remove(&old_origin);
            }
        }
        index
    } else {
        let index = state.entries.borrow().len();
        state.entries.borrow_mut().push(entry);
        state.by_name.borrow_mut().insert(name, index);
        index
    };
    if let Some(origin) = origin {
        state.by_origin.borrow_mut().insert(origin, index);
    }
    Ok(())
}

fn register_script<'s>(
    scope: &mut v8::HandleScope<'s>,
    state: &RegistryState,
    args: v8::FunctionCallbackArguments<'s>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let manifest = args.get(1);
    let tagged = if is_bot_manifest(scope, manifest)? {
        manifest
    } else {
        let define_bot = args.get(3);
        let undefined = v8::undefined(scope).into();
        cb::call(scope, define_bot, undefined, &[manifest], "defineBot")?
    };
    let name = cb::get(scope, tagged, "name")?;
    let description = cb::get(scope, tagged, "description")?;
    let description = if description.is_null_or_undefined() {
        cb::string(scope, "")
    } else {
        description
    };
    let version = cb::get(scope, tagged, "version")?;
    let category = cb::get(scope, tagged, "category")?;
    let tags = cb::get(scope, tagged, "tags")?;
    let origin = args.get(2);
    let origin_key = if origin.is_string() {
        Some(origin.to_rust_string_lossy(scope))
    } else {
        None
    };
    let settings_schema = cb::get(scope, tagged, "settingsSchema")?;
    let create = cb::get(scope, tagged, "create")?;
    let metadata = cb::object(
        scope,
        &[
            ("name", name),
            ("description", description),
            ("version", version),
            ("category", category),
            ("tags", tags),
            ("origin", origin),
            ("settingsSchema", settings_schema),
            ("create", create),
        ],
    )?;
    let metadata = v8::Local::<v8::Object>::try_from(metadata)
        .map_err(|_| cb::type_error(scope, "ScriptMeta could not be created"))?;
    register_metadata(scope, state, metadata, origin_key)?;
    Ok(metadata.into())
}

fn is_bot_manifest<'s>(
    scope: &mut v8::HandleScope<'s>,
    value: v8::Local<'s, v8::Value>,
) -> JsResult<'s, bool> {
    if !value.is_object() || value.is_null() || value.is_function() {
        return Ok(false);
    }
    let marker = cb::get(scope, value, "__rs2b0tManifest")?;
    Ok(marker.is_number() && marker.number_value(scope) == Some(1.0))
}
fn runtime_callback<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = dispatch_runtime(scope, args);
    cb::finish(scope, rv, result);
}

fn dispatch_runtime<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let operation = args.get(0);
    if !operation.is_string() {
        return Err(cb::type_error(
            scope,
            "ScriptRunner runtime operation must be a string",
        ));
    }
    let operation = operation.to_rust_string_lossy(scope);
    let state = state_for_scope(scope)
        .ok_or_else(|| cb::type_error(scope, "ScriptRunner runtime state is not installed"))?;
    let context = state.runtime_context.borrow().as_ref().map(Arc::clone);
    let snapshot = context.as_ref().map(|context| context.snapshot());
    let run_state = snapshot
        .as_ref()
        .map_or(RuntimeRunState::Idle, |snapshot| snapshot.state);
    let requested_generation = generation_arg(args.get(1));
    let generation_matches = match requested_generation {
        None => true,
        Some(requested) => snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.generation == requested),
    };
    match operation.as_str() {
        "isBotManifest" => {
            let is_manifest = is_bot_manifest(scope, args.get(1))?;
            Ok(v8::Boolean::new(scope, is_manifest).into())
        }
        "hasContext" => Ok(v8::Boolean::new(scope, context.is_some()).into()),
        "generation" => {
            let value: v8::Local<'s, v8::Value> = match snapshot {
                Some(snapshot) => v8::BigInt::new_from_u64(scope, snapshot.generation).into(),
                None => v8::null(scope).into(),
            };
            Ok(value)
        }
        "isCurrent" => Ok(v8::Boolean::new(
            scope,
            requested_generation.is_some() && generation_matches,
        )
        .into()),
        "state" => Ok(cb::string(
            scope,
            if generation_matches {
                run_state.as_str()
            } else {
                RuntimeRunState::Stopped.as_str()
            },
        )),
        "active" => Ok(v8::Boolean::new(
            scope,
            generation_matches
                && matches!(
                    run_state,
                    RuntimeRunState::Running | RuntimeRunState::Paused | RuntimeRunState::Stopping
                ),
        )
        .into()),
        "aborted" => Ok(v8::Boolean::new(scope, !generation_matches || run_state.aborted()).into()),
        "stopReason" => match snapshot
            .filter(|_| generation_matches)
            .and_then(|snapshot| snapshot.stop_reason)
        {
            Some(reason) => Ok(cb::string(scope, &reason)),
            None => Ok(v8::null(scope).into()),
        },
        "watchdogHold" => match context
            .as_ref()
            .filter(|_| generation_matches)
            .and_then(|context| context.watchdog_hold())
        {
            Some(hold) => Ok(cb::string(scope, &hold)),
            None => Ok(v8::null(scope).into()),
        },
        "setWatchdogHold" => {
            let Some(context) =
                context.filter(|_| requested_generation.is_some() && generation_matches)
            else {
                return Ok(v8::Boolean::new(scope, false).into());
            };
            let value = args.get(2);
            let hold = if value.is_null_or_undefined() {
                None
            } else {
                Some(cb::to_string(scope, value)?)
            };
            context.set_watchdog_hold(hold);
            Ok(v8::Boolean::new(scope, true).into())
        }
        "sourceIdentity" => match snapshot
            .filter(|_| generation_matches)
            .and_then(|snapshot| snapshot.source_identity)
        {
            Some(identity) => Ok(cb::string(scope, &identity)),
            None => Ok(v8::null(scope).into()),
        },
        "meta" => runtime_metadata(scope, &state, context.as_ref()),
        "pause" => {
            let Some(context) = context else {
                return Ok(v8::Boolean::new(scope, false).into());
            };
            if !context.request_pause() {
                return Ok(v8::Boolean::new(scope, false).into());
            }
            Ok(v8::Boolean::new(scope, true).into())
        }
        "resume" => {
            let Some(context) = context else {
                return Ok(v8::Boolean::new(scope, false).into());
            };
            if !context.request_resume() {
                return Ok(v8::Boolean::new(scope, false).into());
            }
            Ok(v8::Boolean::new(scope, true).into())
        }
        "activityId" => Ok(match lineage_v8::current_activity_id(scope) {
            Some(id) => v8::BigInt::new_from_u64(scope, id.0).into(),
            None => v8::null(scope).into(),
        }),
        "abortWaiters" => {
            if !generation_matches {
                return Ok(v8::undefined(scope).into());
            }
            let target = match lineage_v8::current_activity_id(scope) {
                Some(id) => WaiterScope::Activity(id),
                None if run_state.aborted() => WaiterScope::All,
                None => WaiterScope::Root,
            };
            abort_waiters(scope, target)
        }
        _ => Err(cb::type_error(
            scope,
            "unknown ScriptRunner runtime operation",
        )),
    }
}

fn generation_arg(value: v8::Local<v8::Value>) -> Option<u64> {
    let token = v8::Local::<v8::BigInt>::try_from(value).ok()?;
    let (generation, lossless) = token.u64_value();
    lossless.then_some(generation)
}

#[derive(Clone, Copy)]
enum WaiterScope {
    Root,
    Activity(lineage_v8::ActivityId),
    All,
}

fn abort_waiters<'s>(
    scope: &mut v8::HandleScope<'s>,
    target: WaiterScope,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let global = scope.get_current_context().global(scope);
    let queue_reader = cb::get(scope, global.into(), "__rs2b0t_wait_queue")?;
    let undefined = v8::undefined(scope).into();
    let queue_value = cb::call(scope, queue_reader, undefined, &[], "__rs2b0t_wait_queue")?;
    let Ok(queue) = v8::Local::<v8::Array>::try_from(queue_value) else {
        return Err(cb::type_error(
            scope,
            "Execution wait queue is not an array",
        ));
    };
    let make_aborted = cb::get(scope, global.into(), "__rs2b0t_make_script_aborted")?;
    let activity_value: v8::Local<v8::Value> = match target {
        WaiterScope::Activity(id) => v8::BigInt::new_from_u64(scope, id.0).into(),
        WaiterScope::Root => v8::null(scope).into(),
        WaiterScope::All => v8::undefined(scope).into(),
    };
    let mut retained = 0;
    for index in 0..queue.length() {
        let wait = cb::get_index(scope, queue.into(), index)?;
        let Ok(wait) = v8::Local::<v8::Object>::try_from(wait) else {
            return Err(cb::type_error(
                scope,
                "Execution wait queue contains a non-object",
            ));
        };
        let wait_activity = cb::get(scope, wait.into(), "activityId")?;
        let selected =
            matches!(target, WaiterScope::All) || wait_activity.strict_equals(activity_value);
        if selected {
            let reason = cb::call(
                scope,
                make_aborted,
                undefined,
                &[],
                "__rs2b0t_make_script_aborted",
            )?;
            let reject = cb::get(scope, wait.into(), "reject")?;
            cb::call(scope, reject, undefined, &[reason], "wait.reject")?;
        } else {
            if retained != index {
                let retained_ok =
                    cb::catching(scope, |scope| queue.set_index(scope, retained, wait.into()))?;
                if !retained_ok {
                    return Err(cb::type_error(
                        scope,
                        "Execution wait queue could not retain an entry",
                    ));
                }
            }
            retained += 1;
        }
    }
    let length_key = cb::string(scope, "length");
    let length = cb::num(scope, f64::from(retained));
    let compacted = cb::catching(scope, |scope| {
        queue.set(scope, length_key, length)
    })?;
    if !compacted {
        return Err(cb::type_error(
            scope,
            "Execution wait queue could not compact",
        ));
    }
    Ok(undefined)
}

pub(crate) fn abort_activity_waiters<'s>(
    scope: &mut v8::HandleScope<'s>,
    generation: u64,
    child_id: u64,
) -> JsResult<'s, ()> {
    let state = state_for_scope(scope)
        .ok_or_else(|| cb::type_error(scope, "ScriptRunner runtime state is not installed"))?;
    let context = state.runtime_context.borrow().as_ref().map(Arc::clone);
    let Some(context) = context else {
        return Ok(());
    };
    let snapshot = context.snapshot();
    if snapshot.generation != generation || !context.owns_activity(child_id) {
        return Ok(());
    }
    let _ = abort_waiters(
        scope,
        WaiterScope::Activity(lineage_v8::ActivityId(child_id)),
    )?;
    Ok(())
}

fn runtime_metadata<'s>(
    scope: &mut v8::HandleScope<'s>,
    state: &RegistryState,
    context: Option<&Arc<ScriptRuntimeContext>>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let Some(identity) = context.and_then(|context| context.snapshot().source_identity) else {
        return Ok(v8::null(scope).into());
    };
    let index = identity
        .strip_prefix("catalog:")
        .and_then(|name| state.by_name.borrow().get(name).copied())
        .or_else(|| state.by_origin.borrow().get(&identity).copied());
    let Some(index) = index else {
        return Ok(v8::null(scope).into());
    };
    let entries = state.entries.borrow();
    let Some(entry) = entries.get(index) else {
        return Err(cb::type_error(
            scope,
            "ScriptRegistry index is inconsistent",
        ));
    };
    Ok(v8::Local::new(scope, &entry.metadata).into())
}

fn settings_state_for_scope(scope: &v8::HandleScope) -> Option<Rc<SettingsState>> {
    scope.get_slot::<Rc<SettingsState>>().cloned()
}

fn settings_callback<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = dispatch_settings(scope, args);
    cb::finish(scope, rv, result);
}

fn dispatch_settings<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let operation = args.get(0);
    if !operation.is_string() {
        return Err(cb::type_error(
            scope,
            "SettingsStore operation must be a string",
        ));
    }
    let operation = operation.to_rust_string_lossy(scope);
    let state = settings_state_for_scope(scope)
        .ok_or_else(|| cb::type_error(scope, "SettingsStore native state is not installed"))?;
    match operation.as_str() {
        "saved" => {
            let name = settings_string_arg(scope, args.get(1), "SettingsStore name")?;
            let key = settings_string_arg(scope, args.get(2), "SettingsStore key")?;
            let Some(slot) = state.local_state.as_ref() else {
                return Ok(v8::null(scope).into());
            };
            let identity = slot.identity(LocalStateDomain::ScriptRuntimeSettings);
            match slot.port().runtime_setting_raw(&identity, &name, &key) {
                Ok(Some(raw)) => {
                    let value = v8::String::new(scope, &raw)
                        .ok_or_else(|| cb::type_error(scope, "SettingsStore saved value"))?;
                    Ok(value.into())
                }
                Ok(None) => Ok(v8::undefined(scope).into()),
                Err(error) => Err(settings_error(
                    scope,
                    &format!("SettingsStore read refused: {error:?}"),
                )),
            }
        }
        "save" => {
            let name = settings_string_arg(scope, args.get(1), "SettingsStore name")?;
            let key = settings_string_arg(scope, args.get(2), "SettingsStore key")?;
            let raw = settings_string_arg(scope, args.get(3), "SettingsStore raw value")?;
            let slot = state.local_state.as_ref().ok_or_else(|| {
                settings_error(scope, "SettingsStore local-state service is unavailable")
            })?;
            let identity = slot.identity(LocalStateDomain::ScriptRuntimeSettings);
            let edit = RuntimeSettingsEdit::try_set(identity, None, &name, &key, &raw).map_err(
                |error| settings_error(scope, &format!("SettingsStore save refused: {error:?}")),
            )?;
            match slot.port().submit(DomainEdit::ScriptRuntimeSettings(edit)) {
                SubmitReceipt::Accepted { .. } => {}
                SubmitReceipt::Refused(error) => {
                    return Err(settings_error(
                        scope,
                        &format!("SettingsStore save refused: {error:?}"),
                    ));
                }
            }
            notify_settings_listeners(scope, &state, &name, &key, &raw)
        }
        "subscribe" => {
            let callback = args.get(1);
            let Ok(callback) = v8::Local::<v8::Function>::try_from(callback) else {
                return Err(cb::type_error(
                    scope,
                    "SettingsStore.onChange listener must be a function",
                ));
            };
            let mut listeners = state.listeners.borrow_mut();
            let duplicate = listeners.iter().any(|listener| {
                listener.callback.as_ref().is_some_and(|current| {
                    v8::Local::new(scope, current).strict_equals(callback.into())
                })
            });
            if !duplicate {
                if listeners
                    .iter()
                    .filter(|listener| listener.callback.is_some())
                    .count()
                    >= MAX_SETTINGS_LISTENERS
                {
                    return Err(settings_error(
                        scope,
                        "SettingsStore.onChange listener limit reached",
                    ));
                }
                let order = state.next_order.get();
                let Some(next_order) = order.checked_add(1) else {
                    return Err(settings_error(
                        scope,
                        "SettingsStore listener order exhausted",
                    ));
                };
                state.next_order.set(next_order);
                let listener = SettingsListener {
                    callback: Some(v8::Global::new(scope, callback)),
                    order,
                };
                if let Some(slot) = listeners
                    .iter_mut()
                    .find(|listener| listener.callback.is_none())
                {
                    *slot = listener;
                } else {
                    listeners.push(listener);
                }
            }
            Ok(v8::undefined(scope).into())
        }
        "unsubscribe" => {
            let callback = args.get(1);
            let Ok(callback) = v8::Local::<v8::Function>::try_from(callback) else {
                return Err(cb::type_error(
                    scope,
                    "SettingsStore unsubscribe listener must be a function",
                ));
            };
            if let Some(listener) = state.listeners.borrow_mut().iter_mut().find(|listener| {
                listener.callback.as_ref().is_some_and(|current| {
                    v8::Local::new(scope, current).strict_equals(callback.into())
                })
            }) {
                listener.callback.take();
            }
            Ok(v8::undefined(scope).into())
        }
        _ => Err(cb::type_error(scope, "unknown SettingsStore operation")),
    }
}

fn settings_string_arg<'s>(
    scope: &mut v8::HandleScope<'s>,
    value: v8::Local<'s, v8::Value>,
    label: &str,
) -> JsResult<'s, String> {
    if !value.is_string() {
        return Err(cb::type_error(scope, &format!("{label} must be a string")));
    }
    Ok(value.to_rust_string_lossy(scope))
}

fn settings_error<'s>(scope: &mut v8::HandleScope<'s>, message: &str) -> cb::Throw<'s> {
    let message = v8::String::new(scope, message).unwrap_or_else(|| v8::String::empty(scope));
    cb::Throw::Value(v8::Exception::error(scope, message))
}

const MAX_SETTINGS_LISTENERS: usize = 256;

fn notify_settings_listeners<'s>(
    scope: &mut v8::HandleScope<'s>,
    state: &SettingsState,
    name: &str,
    key: &str,
    raw: &str,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let name =
        v8::String::new(scope, name).ok_or_else(|| cb::type_error(scope, "SettingsStore name"))?;
    let key =
        v8::String::new(scope, key).ok_or_else(|| cb::type_error(scope, "SettingsStore key"))?;
    let raw = v8::String::new(scope, raw)
        .ok_or_else(|| cb::type_error(scope, "SettingsStore raw value"))?;
    let callback_args = [name.into(), key.into(), raw.into()];
    let receiver = v8::undefined(scope).into();
    let mut cursor = None;
    loop {
        let next = {
            let listeners = state.listeners.borrow();
            listeners
                .iter()
                .filter(|listener| {
                    listener.callback.is_some() && cursor.is_none_or(|order| listener.order > order)
                })
                .min_by_key(|listener| listener.order)
                .and_then(|listener| {
                    listener
                        .callback
                        .as_ref()
                        .map(|callback| (listener.order, v8::Local::new(scope, callback)))
                })
        };
        let Some((order, callback)) = next else {
            break;
        };
        cursor = Some(order);
        match cb::call(
            scope,
            callback.into(),
            receiver,
            &callback_args,
            "SettingsStore change listener",
        ) {
            Ok(_) | Err(cb::Throw::Value(_)) => {}
            Err(cb::Throw::Terminated) => return Err(cb::Throw::Terminated),
        }
    }
    Ok(v8::undefined(scope).into())
}

/// Release callback handles and registry metadata before the isolate exits.
pub(crate) fn clear_all(runtime: &mut Runtime) {
    let scope = runtime.deno_runtime().handle_scope();
    if let Some(registry) = scope.get_slot::<Rc<RegistryState>>().cloned() {
        registry.entries.borrow_mut().clear();
        registry.by_name.borrow_mut().clear();
        registry.by_origin.borrow_mut().clear();
        registry.runtime_context.borrow_mut().take();
    }
    if let Some(settings) = scope.get_slot::<Rc<SettingsState>>().cloned() {
        settings.listeners.borrow_mut().clear();
        settings.next_order.set(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runtime() -> Runtime {
        let mut runtime = Runtime::new(rustyscript::RuntimeOptions::default()).unwrap();
        install(&mut runtime, None).unwrap();
        runtime
    }

    #[test]
    fn native_registry_preserves_exact_metadata_and_create_handle() {
        let mut runtime = runtime();
        let result: serde_json::Value = runtime
            .eval(
                r#"(() => {
                    const create = () => ({ marker: 1 });
                    const metadata = { name: 'alpha', create };
                    __rs2b0t_script_registry('register', metadata);
                    return {
                        sameGet: __rs2b0t_script_registry('get', 'alpha') === metadata,
                        sameList: __rs2b0t_script_registry('list')[0] === metadata,
                        sameCreate: __rs2b0t_script_registry('get', 'alpha').create === create,
                        created: __rs2b0t_script_registry('get', 'alpha').create().marker,
                    };
                })()"#,
            )
            .unwrap();
        assert_eq!(result["sameGet"], true);
        assert_eq!(result["sameList"], true);
        assert_eq!(result["sameCreate"], true);
        assert_eq!(result["created"], 1);
    }

    #[test]
    fn native_registry_replaces_in_place_without_reordering() {
        let mut runtime = runtime();
        let result: serde_json::Value = runtime
            .eval(
                r#"(() => {
                    const a = { name: 'a', create() { return 1; } };
                    const b = { name: 'b', create() { return 2; } };
                    const replacement = { name: 'a', create() { return 3; } };
                    __rs2b0t_script_registry('register', a);
                    __rs2b0t_script_registry('register', b);
                    __rs2b0t_script_registry('register', replacement);
                    const list = __rs2b0t_script_registry('list');
                    return [list.length, list[0] === replacement, list[1] === b];
                })()"#,
            )
            .unwrap();
        assert_eq!(result, serde_json::json!([2, true, true]));
    }

    #[test]
    fn native_settings_listener_storage_is_bounded_after_unsubscribe() {
        let mut runtime = runtime();
        runtime
            .eval::<()>(
                r#"for (let index = 0; index < 2048; index++) {
                    const listener = () => {};
                    __rs2b0t_settings_store('subscribe', listener);
                    __rs2b0t_settings_store('unsubscribe', listener);
                }"#,
            )
            .unwrap();
        let scope = runtime.deno_runtime().handle_scope();
        let state = scope.get_slot::<Rc<SettingsState>>().unwrap();
        let listeners = state.listeners.borrow();
        assert_eq!(listeners.len(), 1);
        assert!(listeners.iter().all(|listener| listener.callback.is_none()));
    }
}
