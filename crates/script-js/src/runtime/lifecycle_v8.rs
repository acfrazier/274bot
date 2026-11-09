//! Native runtime registries and run-context handles exposed to compat shims.
//!
//! This is the isolate-local owner for caller-provided runtime descriptors.
//! The static catalog parser remains an input adapter; this registry retains
//! the exact JavaScript metadata object and its original `create` callback.

use crate::materialize::callback_v8::{self as cb, JsResult};
use crate::materialize::lineage_v8;
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
    let compacted = cb::catching(scope, |scope| queue.set(scope, length_key, length))?;
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
        "resolve" => {
            let name = settings_string_arg(scope, args.get(1), "SettingsStore name")?;
            resolve_settings(scope, &state, &name, args.get(2))
        }
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

fn resolve_settings<'s>(
    scope: &mut v8::HandleScope<'s>,
    state: &SettingsState,
    name: &str,
    schema: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let global = scope.get_current_context().global(scope);
    let object_ctor = cb::get(scope, global.into(), "Object")?;
    let schema = if schema.is_null_or_undefined() {
        v8::Object::new(scope).into()
    } else {
        schema
    };
    let keys = cb::call_method(scope, object_ctor, "keys", &[schema], "Object.keys")?;
    let keys = v8::Local::<v8::Array>::try_from(keys)
        .map_err(|_| cb::type_error(scope, "SettingsStore schema keys are not an array"))?;
    let identity = state
        .local_state
        .as_ref()
        .map(|slot| slot.identity(LocalStateDomain::ScriptRuntimeSettings));
    let result = v8::Object::new(scope);
    for index in 0..keys.length() {
        let key = cb::get_index(scope, keys.into(), index)?;
        let key = v8::Local::<v8::String>::try_from(key)
            .map_err(|_| cb::type_error(scope, "SettingsStore schema key is not a string"))?;
        let key_value: v8::Local<v8::Value> = key.into();
        let key_text = key.to_rust_string_lossy(scope);
        let definition = cb::get(scope, schema, &key_text)?;
        let raw = match (state.local_state.as_ref(), identity.as_ref()) {
            (Some(slot), Some(identity)) => slot
                .port()
                .runtime_setting_raw(identity, name, &key_text)
                .map_err(|error| {
                    settings_error(scope, &format!("SettingsStore read refused: {error:?}"))
                })?,
            _ => None,
        };
        let value = if let Some(raw) = raw {
            parse_setting_value(scope, definition, &raw)?
        } else {
            cb::get(scope, definition, "default")?
        };
        cb::assign(scope, result.into(), key_value, value)?;
    }
    Ok(result.into())
}

fn setting_options<'s>(
    scope: &mut v8::HandleScope<'s>,
    definition: v8::Local<'s, v8::Value>,
) -> JsResult<'s, Option<v8::Local<'s, v8::Array>>> {
    let options = cb::get(scope, definition, "options")?;
    if !cb::truthy(scope, options) {
        return Ok(None);
    }
    let options = v8::Local::<v8::Array>::try_from(options)
        .map_err(|_| cb::type_error(scope, "SettingsStore options must be an array"))?;
    Ok((options.length() > 0).then_some(options))
}

fn trimmed_setting_value<'s>(
    scope: &mut v8::HandleScope<'s>,
    raw: &str,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let raw = v8::String::new(scope, raw)
        .ok_or_else(|| cb::type_error(scope, "SettingsStore raw value"))?;
    cb::call_method(scope, raw.into(), "trim", &[], "SettingsStore value trim")
}

fn find_setting_option<'s>(
    scope: &mut v8::HandleScope<'s>,
    options: v8::Local<'s, v8::Array>,
    wanted: v8::Local<'s, v8::Value>,
) -> JsResult<'s, Option<v8::Local<'s, v8::Value>>> {
    for index in 0..options.length() {
        let option = cb::get_index(scope, options.into(), index)?;
        let lowered = cb::call_method(
            scope,
            option,
            "toLowerCase",
            &[],
            "SettingsStore option lowercase",
        )?;
        if lowered.strict_equals(wanted) {
            return Ok(Some(option));
        }
    }
    Ok(None)
}

fn parse_setting_value<'s>(
    scope: &mut v8::HandleScope<'s>,
    definition: v8::Local<'s, v8::Value>,
    raw: &str,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let kind = cb::get(scope, definition, "type")?;
    if !kind.is_string() {
        return cb::get(scope, definition, "default");
    }
    let kind = kind.to_rust_string_lossy(scope);
    match kind.as_str() {
        "boolean" => {
            let trimmed = trimmed_setting_value(scope, raw)?;
            let lowered = cb::call_method(
                scope,
                trimmed,
                "toLowerCase",
                &[],
                "SettingsStore boolean lowercase",
            )?;
            let normalized = lowered.to_rust_string_lossy(scope);
            let value = matches!(normalized.as_str(), "true" | "1" | "yes");
            Ok(v8::Boolean::new(scope, value).into())
        }
        "number" => {
            let raw = v8::String::new(scope, raw)
                .ok_or_else(|| cb::type_error(scope, "SettingsStore raw value"))?;
            let number = cb::number(scope, raw.into())?;
            if !number.is_finite() {
                return cb::get(scope, definition, "default");
            }
            let min = cb::get(scope, definition, "min")?;
            let mut value = if min.is_undefined() {
                number
            } else {
                setting_math_max(cb::number(scope, min)?, number)
            };
            let max = cb::get(scope, definition, "max")?;
            if !max.is_undefined() {
                value = setting_math_min(cb::number(scope, max)?, value);
            }
            Ok(v8::Number::new(scope, value).into())
        }
        "string" => {
            let trimmed = trimmed_setting_value(scope, raw)?;
            if let Some(options) = setting_options(scope, definition)? {
                let wanted = cb::call_method(
                    scope,
                    trimmed,
                    "toLowerCase",
                    &[],
                    "SettingsStore string lowercase",
                )?;
                if let Some(option) = find_setting_option(scope, options, wanted)? {
                    Ok(option)
                } else {
                    cb::get(scope, definition, "default")
                }
            } else {
                Ok(trimmed)
            }
        }
        "string[]" => parse_string_list(scope, definition, raw),
        "tile" => parse_tile_setting(scope, definition, raw),
        _ => cb::get(scope, definition, "default"),
    }
}

fn parse_string_list<'s>(
    scope: &mut v8::HandleScope<'s>,
    definition: v8::Local<'s, v8::Value>,
    raw: &str,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let options = setting_options(scope, definition)?;
    let output = v8::Array::new(scope, 0);
    let mut written = 0_u32;
    for part in raw.split(',') {
        let value = trimmed_setting_value(scope, part)?;
        let value = v8::Local::<v8::String>::try_from(value)
            .map_err(|_| cb::type_error(scope, "SettingsStore list value is not a string"))?;
        if value.length() == 0 {
            continue;
        }
        let value: v8::Local<v8::Value> = if let Some(options) = options {
            let wanted = cb::call_method(
                scope,
                value.into(),
                "toLowerCase",
                &[],
                "SettingsStore list lowercase",
            )?;
            let Some(option) = find_setting_option(scope, options, wanted)? else {
                continue;
            };
            option
        } else {
            value.into()
        };
        if written == u32::MAX {
            return Err(cb::type_error(
                scope,
                "SettingsStore list exceeds the V8 array limit",
            ));
        }
        if !cb::catching(scope, |scope| output.set_index(scope, written, value))? {
            return Err(cb::type_error(
                scope,
                "could not materialize SettingsStore list",
            ));
        }
        written += 1;
    }
    Ok(output.into())
}

fn parse_tile_setting<'s>(
    scope: &mut v8::HandleScope<'s>,
    definition: v8::Local<'s, v8::Value>,
    raw: &str,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    if raw.split(',').count() < 2 {
        return cb::get(scope, definition, "default");
    }
    let mut coordinates = [0.0; 3];
    for (index, part) in raw.split(',').enumerate() {
        let trimmed = trimmed_setting_value(scope, part)?;
        let number = cb::number(scope, trimmed)?;
        if !number.is_finite() {
            return cb::get(scope, definition, "default");
        }
        if let Some(coordinate) = coordinates.get_mut(index) {
            *coordinate = number;
        }
    }
    let tile = v8::Object::new(scope);
    let x_key = cb::string(scope, "x");
    let x = v8::Number::new(scope, coordinates[0]);
    cb::assign(scope, tile.into(), x_key, x.into())?;
    let z_key = cb::string(scope, "z");
    let z = v8::Number::new(scope, coordinates[1]);
    cb::assign(scope, tile.into(), z_key, z.into())?;
    let level_key = cb::string(scope, "level");
    let level = v8::Number::new(scope, coordinates[2]);
    cb::assign(scope, tile.into(), level_key, level.into())?;
    Ok(tile.into())
}

fn setting_math_max(left: f64, right: f64) -> f64 {
    if left.is_nan() || right.is_nan() {
        f64::NAN
    } else if left == 0.0 && right == 0.0 {
        if left.is_sign_negative() && right.is_sign_negative() {
            -0.0
        } else {
            0.0
        }
    } else {
        left.max(right)
    }
}

fn setting_math_min(left: f64, right: f64) -> f64 {
    if left.is_nan() || right.is_nan() {
        f64::NAN
    } else if left == 0.0 && right == 0.0 {
        if left.is_sign_negative() || right.is_sign_negative() {
            -0.0
        } else {
            0.0
        }
    } else {
        left.min(right)
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
        cb::install_ops(&mut runtime).unwrap();
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
    fn native_settings_resolver_uses_schema_defaults_not_host_bag() {
        let mut runtime = runtime();
        let result: serde_json::Value = runtime
            .eval(
                r#"(() => {
                    const inherited = { inherited: 'parent' };
                    const bag = Object.create(inherited);
                    bag.food = 'Shark';
                    bag.enabled = false;
                    globalThis.__rs2b0t_host = { settingsBag: bag };
                    const schema = {
                        food: { default: 'Lobster' },
                        enabled: { default: true },
                        missing: { default: 'none' },
                        inherited: { default: 'not copied' },
                    };
                    const resolved = __rs2b0t_settings_store('resolve', 'Test', schema);
                    return {
                        food: resolved.food,
                        enabled: resolved.enabled,
                        missing: resolved.missing,
                        inherited: resolved.inherited,
                        keys: Object.keys(resolved),
                        empty: Object.keys(__rs2b0t_settings_store('resolve', 'Test', null)).length === 0,
                    };
                })()"#,
            )
            .unwrap();
        assert_eq!(
            result,
            serde_json::json!({
                "food": "Lobster",
                "enabled": true,
                "missing": "none",
                "inherited": "not copied",
                "keys": ["food", "enabled", "missing", "inherited"],
                "empty": true,
            })
        );
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

#[cfg(all(test, feature = "memory-profile"))]
mod settings_resolve_v8_tests {
    use std::process::Command;

    use rustyscript::deno_core::v8;
    use rustyscript::{Runtime, RuntimeOptions};
    use serde_json::json;

    const FAST_CHILD: &str = "SCRIPT_SETTINGS_RESOLVE_FAST_TEST_CHILD";
    const FAST_TEST: &str =
        "lifecycle_v8::settings_resolve_v8_tests::settings_resolve_objects_have_fast_properties";
    const HEAP_CHILD: &str = "SCRIPT_SETTINGS_RESOLVE_HEAP_TEST_CHILD";
    const HEAP_TEST: &str =
        "lifecycle_v8::settings_resolve_v8_tests::settings_resolve_objects_have_bounded_heap";
    const SETUP: &str = r#"
globalThis.__rs2b0t_host = { settingsBag: { food: 'Shark', enabled: false } };
globalThis.__resolveSchema = {
    food: { type: 'string', default: 'Lobster' },
    enabled: { type: 'boolean', default: true },
    missing: { type: 'string', default: 'none' },
    tile: { type: 'tile', default: { x: 1, z: 2, level: 0 } },
};
"#;
    struct SettingsRawPort;

    impl api::local_state::LocalStatePort for SettingsRawPort {
        fn lease(
            &self,
            identity: &api::local_state::DomainIdentity,
        ) -> Result<api::local_state::DomainLease, api::local_state::ErrorCode> {
            Err(api::local_state::ErrorCode::NotReady {
                domain: identity.domain(),
            })
        }

        fn runtime_setting_raw(
            &self,
            identity: &api::local_state::DomainIdentity,
            namespace: &str,
            key: &str,
        ) -> Result<Option<String>, api::local_state::ErrorCode> {
            if identity.domain() != api::local_state::LocalStateDomain::ScriptRuntimeSettings {
                return Err(api::local_state::ErrorCode::Validation {
                    domain: identity.domain(),
                    reason: api::local_state::ValidationReason::Identity,
                });
            }
            Ok((namespace == "Test" && key == "tile").then(|| "3221,3218,2".to_owned()))
        }

        fn submit(&self, edit: api::local_state::DomainEdit) -> api::local_state::SubmitReceipt {
            api::local_state::SubmitReceipt::Refused(api::local_state::ErrorCode::Busy {
                domain: edit.identity().domain(),
            })
        }

        fn observe(
            &self,
            _request: api::identity::RequestId,
        ) -> Option<api::local_state::WriteSettlement> {
            None
        }
    }

    impl script::local_state::StatePort for SettingsRawPort {
        fn lease_version(
            &self,
            identity: &api::local_state::DomainIdentity,
        ) -> Result<api::local_state::StoreVersion, api::local_state::ErrorCode> {
            Err(api::local_state::ErrorCode::NotReady {
                domain: identity.domain(),
            })
        }
    }

    fn setup() -> Runtime {
        crate::runtime::platform::bootstrap_test_v8_platform();
        let mut runtime = Runtime::new(RuntimeOptions::default()).unwrap();
        crate::materialize::callback_v8::install_ops(&mut runtime).unwrap();
        let state = std::sync::Arc::new(script::local_state::SlotState::new(
            std::sync::Arc::new(SettingsRawPort),
            api::local_state::AccountKey::try_new("local-289", "runtime").unwrap(),
            api::local_state::ContentScope::try_new(
                api::selected::ClientRevision::R289,
                "settings-resolve",
            )
            .unwrap(),
        ));
        super::install(&mut runtime, Some(state)).unwrap();
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
            .expect("run isolated SettingsStore V8 test");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            output.status.success() && stdout.contains("1 passed"),
            "isolated SettingsStore V8 test failed\nstdout: {stdout}\nstderr: {stderr}"
        );
        false
    }

    fn collected_used_heap(runtime: &mut Runtime) -> usize {
        let isolate = runtime.deno_runtime().v8_isolate();
        isolate.low_memory_notification();
        isolate.get_heap_statistics().used_heap_size()
    }

    #[test]
    fn settings_resolve_objects_have_fast_properties() {
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
    const haveSameMap = new Function('left', 'right', 'return %HaveSameMap(left, right)');
    const first = __rs2b0t_settings_store('resolve', 'Test', __resolveSchema);
    const second = __rs2b0t_settings_store('resolve', 'Test', __resolveSchema);
    return {
        fast: hasFast(first) && hasFast(second) && hasFast(first.tile) && hasFast(second.tile),
        sameMap: haveSameMap(first, second) && haveSameMap(first.tile, second.tile),
        keys: Object.keys(first),
        values: [first.food, first.enabled, first.missing, first.tile.x, first.tile.z, first.tile.level],
        plain: Object.getPrototypeOf(first) === Object.prototype
            && Object.getPrototypeOf(first.tile) === Object.prototype,
    };
})()
"#,
            )
            .unwrap();
        assert_eq!(
            facts,
            json!({
                "fast": true,
                "sameMap": true,
                "keys": ["food", "enabled", "missing", "tile"],
                "values": ["Lobster", true, "none", 3221, 3218, 2],
                "plain": true,
            })
        );
    }

    #[test]
    fn settings_resolve_objects_have_bounded_heap() {
        if !in_child(HEAP_CHILD, HEAP_TEST) {
            return;
        }
        let mut runtime = setup();
        runtime
            .eval::<()>(
                "for (let index = 0; index < 1000; index++) globalThis.__lastResolved = __rs2b0t_settings_store('resolve', 'Test', __resolveSchema); globalThis.__lastResolved = null;",
            )
            .unwrap();
        let baseline = collected_used_heap(&mut runtime);
        runtime
            .eval::<()>(
                "for (let index = 0; index < 25000; index++) globalThis.__lastResolved = __rs2b0t_settings_store('resolve', 'Test', __resolveSchema); globalThis.__lastResolved = null;",
            )
            .unwrap();
        let after = collected_used_heap(&mut runtime);
        let growth = after.saturating_sub(baseline);
        const LIMIT: usize = 64 * 1024;
        println!(
            "SettingsStore resolve post-GC V8 heap baseline={baseline} after={after} growth={growth} limit={LIMIT}"
        );
        assert!(
            growth <= LIMIT,
            "repeated SettingsStore resolver calls grew post-GC V8 used heap by {growth} bytes; limit {LIMIT}"
        );
    }
}
