//! One per-isolate registry for compat event listeners and native event delivery.

use std::cell::RefCell;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use rustyscript::deno_core::v8;
use rustyscript::Runtime;

use crate::callback_v8;
use crate::lineage_v8;

const INVOKE_LISTENER: &str = "__rs2b0t_invoke_native_event_listener";
pub(crate) const BOT_TICK_EVENT: &str = "__rs2b0t_internal_tick__";

struct Listener {
    id: u32,
    event: v8::Global<v8::Value>,
    callback: v8::Global<v8::Value>,
    is_frame: bool,
    activity_id: Option<lineage_v8::ActivityId>,
}

struct Registry {
    listeners: Vec<Option<Listener>>,
    next_id: u32,
    dispatch_depth: usize,
    frame_interest: Option<Arc<AtomicUsize>>,
}

impl Default for Registry {
    fn default() -> Self {
        Self {
            listeners: Vec::new(),
            next_id: 1,
            dispatch_depth: 0,
            frame_interest: None,
        }
    }
}

thread_local! {
    static REGISTRY: RefCell<Registry> = RefCell::new(Registry::default());
}

/// Install the native event bindings into one runtime.
pub(crate) fn install(
    runtime: &mut Runtime,
    frame_interest: Arc<AtomicUsize>,
) -> Result<(), String> {
    clear_all();
    REGISTRY.with(|registry| {
        let mut registry = registry.borrow_mut();
        registry.frame_interest = Some(frame_interest);
        registry.next_id = 1;
    });

    let result = (|| {
        callback_v8::install(runtime, "__rs2b0t_native_event_on", on)?;
        callback_v8::install(runtime, "__rs2b0t_native_event_off", off)?;
        callback_v8::install(
            runtime,
            "__rs2b0t_native_event_batch_dispatch",
            batch_dispatch,
        )?;
        Ok(())
    })();
    if result.is_err() {
        clear_all();
    }
    result
}

/// Retain a callback unless the event/callback pair already exists.
pub(crate) fn add_callback<'s>(
    scope: &mut v8::HandleScope<'s>,
    event: v8::Local<'s, v8::Value>,
    callback: v8::Local<'s, v8::Value>,
) -> u32 {
    let is_frame = is_frame_event(scope, event);
    let activity_id = lineage_v8::current_activity_id(scope);
    REGISTRY.with(|registry| {
        let mut registry = registry.borrow_mut();
        if let Some(existing) = registry.listeners.iter().flatten().find(|listener| {
            v8::Local::new(scope, &listener.event).strict_equals(event)
                && v8::Local::new(scope, &listener.callback).strict_equals(callback)
        }) {
            return existing.id;
        }

        let id = allocate_id(&mut registry);
        let listener = Listener {
            id,
            event: v8::Global::new(scope, event),
            callback: v8::Global::new(scope, callback),
            activity_id,
            is_frame,
        };
        registry.listeners.push(Some(listener));
        if is_frame {
            if let Some(interest) = &registry.frame_interest {
                interest.fetch_add(1, Ordering::Relaxed);
            }
        }
        id
    })
}

/// Remove one callback by its registration id.
pub(crate) fn remove_callback(id: u32) {
    REGISTRY.with(|registry| {
        let mut registry = registry.borrow_mut();
        let Some(index) = registry
            .listeners
            .iter()
            .position(|entry| entry.as_ref().is_some_and(|listener| listener.id == id))
        else {
            return;
        };
        remove_at(&mut registry, index);
        compact_if_idle(&mut registry);
    });
}

/// Remove the event/callback pair used by `events.off`.
pub(crate) fn remove_callback_for<'s>(
    scope: &mut v8::HandleScope<'s>,
    event: v8::Local<'s, v8::Value>,
    callback: v8::Local<'s, v8::Value>,
) {
    REGISTRY.with(|registry| {
        let mut registry = registry.borrow_mut();
        let Some(index) = registry.listeners.iter().position(|entry| {
            entry.as_ref().is_some_and(|listener| {
                v8::Local::new(scope, &listener.event).strict_equals(event)
                    && v8::Local::new(scope, &listener.callback).strict_equals(callback)
            })
        }) else {
            return;
        };
        remove_at(&mut registry, index);
        compact_if_idle(&mut registry);
    });
}

/// Deliver a no-argument event to all listeners in registration order.
pub(crate) fn emit_no_args(runtime: &mut Runtime, event: &str) -> Result<(), String> {
    let context = runtime.deno_runtime().main_context();
    let mut scope = runtime.deno_runtime().handle_scope();
    let global = context.open(&mut scope).global(&mut scope);
    emit_value(&mut scope, event, None, global)
}

/// Drop every callback after one loaded script stops while retaining the
/// per-isolate frame-interest handle for a later Load in the same isolate.
pub(crate) fn clear_listeners() {
    REGISTRY.with(|registry| {
        let mut registry = registry.borrow_mut();
        registry.listeners.clear();
        if let Some(interest) = &registry.frame_interest {
            interest.store(0, Ordering::Relaxed);
        }
    });
}

/// Drop every callback before its isolate is destroyed.
pub(crate) fn clear_all() {
    clear_listeners();
    REGISTRY.with(|registry| {
        let mut registry = registry.borrow_mut();
        registry.next_id = 1;
        registry.dispatch_depth = 0;
        registry.frame_interest.take();
    });
}

fn on<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let event = args.get(0);
    if !event.is_string()
        || !matches!(
            event.to_rust_string_lossy(scope).as_str(),
            "chat.message"
                | "skill.xp"
                | "skill.level"
                | "inventory.changed"
                | "script.finish"
                | "tick"
                | "frame"
                | "__rs2b0t_internal_tick__"
        )
    {
        let error = callback_v8::type_error(scope, "events.on: unsupported event name");
        callback_v8::finish(scope, rv, Err(error));
        return;
    }
    let id = add_callback(scope, event, args.get(1));
    let value = v8::Number::new(scope, f64::from(id)).into();
    callback_v8::finish(scope, rv, Ok(value));
}

fn off<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue,
) {
    if args.length() > 1 {
        remove_callback_for(scope, args.get(0), args.get(1));
    } else if let Some(id) = callback_id(scope, args.get(0)) {
        remove_callback(id);
    }
    rv.set(v8::undefined(scope).into());
}

fn batch_dispatch<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = dispatch_batch(scope, args.get(0)).map(|()| v8::undefined(scope).into());
    let result = result.map_err(|error| callback_v8::type_error(scope, &error));
    callback_v8::finish(scope, rv, result);
}

fn dispatch_batch<'s>(
    scope: &mut v8::HandleScope<'s>,
    batch: v8::Local<'s, v8::Value>,
) -> Result<(), String> {
    let batch = v8::Local::<v8::Array>::try_from(batch)
        .map_err(|_| "native event batch is not an array")?;
    let global = scope.get_current_context().global(scope);
    let type_key = v8::String::new(scope, "type").ok_or("native event type key")?;
    let payload_key = v8::String::new(scope, "payload").ok_or("native event payload key")?;
    for index in 0..batch.length() {
        let value = batch
            .get_index(scope, index)
            .ok_or("native event batch entry")?;
        let event = v8::Local::<v8::Object>::try_from(value)
            .map_err(|_| "native event entry is not an object")?;
        let event_type = event
            .get(scope, type_key.into())
            .ok_or("native event type")?;
        if !event_type.is_string() {
            return Err("native event type is not a string".into());
        }
        let event_type = event_type.to_rust_string_lossy(scope);
        let payload = event
            .get(scope, payload_key.into())
            .ok_or("native event payload")?;
        emit_value(scope, &event_type, Some(payload), global)?;
    }
    Ok(())
}

fn emit_value<'s>(
    scope: &mut v8::HandleScope<'s>,
    event: &str,
    payload: Option<v8::Local<'s, v8::Value>>,
    global: v8::Local<'s, v8::Object>,
) -> Result<(), String> {
    let event_key = v8::String::new(scope, event).ok_or("event name")?;
    let event_key: v8::Local<v8::Value> = event_key.into();
    let helper_key = v8::String::new(scope, INVOKE_LISTENER).ok_or("event invoker name")?;
    let helper = global
        .get(scope, helper_key.into())
        .ok_or("event invoker missing")?;
    let helper = v8::Local::<v8::Function>::try_from(helper)
        .map_err(|_| "event invoker is not a function")?;
    let receiver = v8::undefined(scope).into();
    let has_payload = v8::Boolean::new(scope, payload.is_some()).into();
    let payload = payload.unwrap_or_else(|| v8::undefined(scope).into());

    let mut index = 0;
    let _dispatch = DispatchGuard::enter();
    loop {
        let listener = REGISTRY.with(|registry| {
            let registry = registry.borrow();
            while index < registry.listeners.len() {
                let current = index;
                index += 1;
                let Some(listener) = registry.listeners[current].as_ref() else {
                    continue;
                };
                if v8::Local::new(scope, &listener.event).strict_equals(event_key) {
                    return Some((
                        v8::Local::new(scope, &listener.callback),
                        listener.activity_id,
                    ));
                }
            }
            None
        });
        let Some((callback, activity_id)) = listener else {
            break;
        };
        let arguments = [callback, payload, has_payload];
        let invoked = match activity_id {
            Some(activity_id) => lineage_v8::enter_activity(scope, activity_id, |scope| {
                helper.call(scope, receiver, &arguments)
            }),
            None => Ok(helper.call(scope, receiver, &arguments)),
        };
        if !matches!(invoked, Ok(Some(_))) {
            return Err("event listener invocation failed".to_string());
        }
    }
    Ok(())
}

fn callback_id(scope: &mut v8::HandleScope<'_>, value: v8::Local<v8::Value>) -> Option<u32> {
    if !value.is_number() {
        return None;
    }
    let id = value.number_value(scope)?;
    (id.is_finite() && id.fract() == 0.0 && (1.0..=f64::from(u32::MAX)).contains(&id))
        .then_some(id as u32)
}

fn is_frame_event(scope: &mut v8::HandleScope<'_>, event: v8::Local<v8::Value>) -> bool {
    event.is_string() && event.to_rust_string_lossy(scope) == "frame"
}

fn allocate_id(registry: &mut Registry) -> u32 {
    loop {
        let id = registry.next_id.max(1);
        registry.next_id = id.wrapping_add(1).max(1);
        if registry
            .listeners
            .iter()
            .flatten()
            .all(|listener| listener.id != id)
        {
            return id;
        }
    }
}

fn remove_at(registry: &mut Registry, index: usize) {
    let Some(listener) = registry.listeners[index].take() else {
        return;
    };
    if listener.is_frame {
        if let Some(interest) = &registry.frame_interest {
            let _ = interest.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |count| {
                count.checked_sub(1)
            });
        }
    }
}

fn compact_if_idle(registry: &mut Registry) {
    if registry.dispatch_depth == 0 {
        registry.listeners.retain(Option::is_some);
    }
}

struct DispatchGuard;

impl DispatchGuard {
    fn enter() -> Self {
        REGISTRY.with(|registry| registry.borrow_mut().dispatch_depth += 1);
        Self
    }
}

impl Drop for DispatchGuard {
    fn drop(&mut self) {
        REGISTRY.with(|registry| {
            let mut registry = registry.borrow_mut();
            registry.dispatch_depth = registry.dispatch_depth.saturating_sub(1);
            if registry.dispatch_depth == 0 {
                registry.listeners.retain(Option::is_some);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use std::mem::size_of;

    use super::Listener;

    #[test]
    fn retained_listener_record_stays_within_native_memory_budget() {
        assert!(
            size_of::<Option<Listener>>() <= 256,
            "retained listener record uses {} bytes",
            size_of::<Option<Listener>>()
        );
    }

    #[test]
    fn script_stop_clears_callbacks_but_keeps_frame_interest_for_reload() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        use super::{clear_all, clear_listeners, REGISTRY};

        let interest = Arc::new(AtomicUsize::new(3));
        REGISTRY.with(|registry| {
            let mut registry = registry.borrow_mut();
            registry.frame_interest = Some(Arc::clone(&interest));
            registry.next_id = 7;
        });

        clear_listeners();

        assert_eq!(interest.load(Ordering::Relaxed), 0);
        REGISTRY.with(|registry| {
            let registry = registry.borrow();
            assert!(registry.frame_interest.is_some());
            assert_eq!(registry.next_id, 7);
            assert!(registry.listeners.is_empty());
        });
        clear_all();
    }
    #[test]
    fn frame_interest_tracks_unique_subscriptions_and_dispatch() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        use rustyscript::{Runtime, RuntimeOptions};

        use super::{clear_all, emit_no_args, install};

        crate::platform::bootstrap_test_v8_platform();
        let interest = Arc::new(AtomicUsize::new(0));
        let mut runtime = Runtime::new(RuntimeOptions::default()).unwrap();
        install(&mut runtime, Arc::clone(&interest)).unwrap();
        runtime.eval::<()>(crate::shim::PRELUDE).unwrap();
        runtime
            .eval::<()>(
                r#"
globalThis.__frame_calls = 0;
const callback = () => { globalThis.__frame_calls += 1; };
globalThis.__frame_id = globalThis.__rs2b0t_native_event_on('frame', callback);
globalThis.__frame_duplicate_id = globalThis.__rs2b0t_native_event_on('frame', callback);
"#,
            )
            .unwrap();
        assert_eq!(interest.load(Ordering::Relaxed), 1);
        let duplicate = runtime
            .eval::<bool>("globalThis.__frame_id === globalThis.__frame_duplicate_id")
            .unwrap();
        assert!(duplicate, "duplicate frame callback shares one record");

        emit_no_args(&mut runtime, "frame").unwrap();
        assert_eq!(
            runtime.eval::<usize>("globalThis.__frame_calls").unwrap(),
            1
        );
        runtime
            .eval::<()>("globalThis.__rs2b0t_native_event_off(globalThis.__frame_id)")
            .unwrap();
        assert_eq!(interest.load(Ordering::Relaxed), 0);
        emit_no_args(&mut runtime, "frame").unwrap();
        assert_eq!(
            runtime.eval::<usize>("globalThis.__frame_calls").unwrap(),
            1
        );
        clear_all();
    }

    #[test]
    fn unknown_event_subscriptions_are_refused() {
        use std::sync::atomic::AtomicUsize;
        use std::sync::Arc;

        use rustyscript::{Runtime, RuntimeOptions};

        use super::{clear_all, install};

        crate::platform::bootstrap_test_v8_platform();
        let mut runtime = Runtime::new(RuntimeOptions::default()).unwrap();
        install(&mut runtime, Arc::new(AtomicUsize::new(0))).unwrap();
        runtime.eval::<()>(crate::shim::PRELUDE).unwrap();
        let message: String = runtime
            .eval(
                "(() => { try { __rs2b0t_native_event_on('varp.changed', () => {}); return ''; } catch (error) { return error.message; } })()",
            )
            .unwrap();
        assert_eq!(message, "events.on: unsupported event name");
        clear_all();
    }
}
#[cfg(all(test, feature = "memory-profile"))]
mod v8_tests {
    use std::process::Command;
    use std::sync::atomic::AtomicUsize;
    use std::sync::Arc;

    use rustyscript::deno_core::v8;
    use rustyscript::{Runtime, RuntimeOptions};
    use serde_json::json;

    use super::{clear_all, install};
    use crate::events::NativeEvent;
    use crate::snapshot::dispatch_native_events;

    const FAST_CHILD: &str = "SCRIPT_EVENT_FAST_TEST_CHILD";
    const FAST_TEST: &str =
        "bindings::event_callbacks::v8_tests::event_payload_has_fast_properties";
    const HEAP_CHILD: &str = "SCRIPT_EVENT_HEAP_TEST_CHILD";
    const HEAP_TEST: &str =
        "bindings::event_callbacks::v8_tests::event_payload_heap_is_bounded_after_gc";

    fn setup() -> Runtime {
        crate::platform::bootstrap_test_v8_platform();
        let mut runtime = Runtime::new(RuntimeOptions::default()).unwrap();
        install(&mut runtime, Arc::new(AtomicUsize::new(0))).unwrap();
        runtime.eval::<()>(crate::shim::PRELUDE).unwrap();
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
            .expect("run isolated V8 event-payload test");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            output.status.success() && stdout.contains("1 passed"),
            "isolated V8 event-payload test failed\nstdout: {stdout}\nstderr: {stderr}"
        );
        false
    }

    fn collected_used_heap(runtime: &mut Runtime) -> usize {
        let isolate = runtime.deno_runtime().v8_isolate();
        isolate.low_memory_notification();
        isolate.get_heap_statistics().used_heap_size()
    }


    #[test]
    fn event_payload_has_fast_properties() {
        if !in_child(FAST_CHILD, FAST_TEST) {
            return;
        }
        v8::V8::set_flags_from_string("--allow-natives-syntax");
        let mut runtime = setup();
        runtime
            .eval::<()>(
                r#"
globalThis.__nativeEventPayloads = [];
globalThis.__rs2b0t_native_event_batch_dispatch_original =
    globalThis.__rs2b0t_native_event_batch_dispatch;
globalThis.__rs2b0t_native_event_batch_dispatch = (events) => {
    globalThis.__nativeEventRecords = events;
    globalThis.__rs2b0t_native_event_batch_dispatch_original(events);
};
for (const type of ['chat.message', 'skill.xp', 'skill.level', 'script.finish', 'inventory.changed']) {
    globalThis.__rs2b0t_native_event_on(type, payload => {
        globalThis.__nativeEventPayloads.push(payload);
    });
}
globalThis.__nativeEventShapeFacts = () => {
    const fast = new Function('value', 'return %HasFastProperties(value)');
    return {
        records: globalThis.__nativeEventRecords.map(event => ({
            type: event.type,
            wrapperFast: fast(event),
            payloadFast: fast(event.payload),
            payloadKeys: Object.keys(event.payload),
        })),
        payloadCount: globalThis.__nativeEventPayloads.length,
    };
};
"#,
            )
            .unwrap();
        let events = sample_events();
        dispatch_native_events(&mut runtime, &events).unwrap();
        flush_pending_events(&mut runtime);
        let facts: serde_json::Value = runtime.eval("__nativeEventShapeFacts()").unwrap();
        assert_eq!(
            facts,
            json!({
                "records": [
                    {
                        "type": "chat.message",
                        "wrapperFast": true,
                        "payloadFast": true,
                        "payloadKeys": ["type", "username", "text"]
                    },
                    {
                        "type": "skill.xp",
                        "wrapperFast": true,
                        "payloadFast": true,
                        "payloadKeys": ["skill", "name", "xp", "delta"]
                    },
                    {
                        "type": "skill.level",
                        "wrapperFast": true,
                        "payloadFast": true,
                        "payloadKeys": ["skill", "name", "level", "previous"]
                    },
                    {
                        "type": "script.finish",
                        "wrapperFast": true,
                        "payloadFast": true,
                        "payloadKeys": ["reason"]
                    },
                    {
                        "type": "inventory.changed",
                        "wrapperFast": true,
                        "payloadFast": true,
                        "payloadKeys": [
                            "slot", "id", "name", "count", "previousId", "previousCount"
                        ]
                    }
                ],
                "payloadCount": 5
            }),
            "production native-event wrappers and payloads retain fixed V8 shapes"
        );
        clear_all();
    }

    fn sample_events() -> [NativeEvent; 5] {
        [
            NativeEvent::ChatMessage {
                type_: 0,
                username: Some("Ada".into()),
                text: "hello".into(),
            },
            NativeEvent::SkillXp {
                skill: 5,
                name: "prayer".into(),
                xp: 1000,
                delta: 20,
            },
            NativeEvent::SkillLevel {
                skill: 5,
                name: "prayer".into(),
                level: 3,
                previous: 2,
            },
            NativeEvent::ScriptFinish {
                reason: "finished".into(),
            },
            NativeEvent::InventoryChanged {
                slot: 2,
                id: 995,
                name: Some("Coins".into()),
                count: 7,
                previous_id: 995,
                previous_count: 6,
            },
        ]
    }
    fn flush_pending_events(runtime: &mut Runtime) {
        runtime
            .eval::<()>(
                "globalThis.__rs2b0t_dispatch_native_events(globalThis.__rs2b0t_pending_native_event_batch.splice(0))",
            )
            .unwrap();
    }

    #[test]
    fn event_payload_heap_is_bounded_after_gc() {
        if !in_child(HEAP_CHILD, HEAP_TEST) {
            return;
        }
        v8::V8::set_flags_from_string("--allow-natives-syntax");
        let mut runtime = setup();
        runtime
            .eval::<()>(
                r#"
globalThis.__event_payload_count = 0;
for (const type of ['chat.message', 'skill.xp', 'skill.level', 'script.finish', 'inventory.changed']) {
    globalThis.__rs2b0t_native_event_on(type, () => {
        globalThis.__event_payload_count += 1;
    });
}
"#,
            )
            .unwrap();
        let events = sample_events();
        dispatch_native_events(&mut runtime, &events).unwrap();
        flush_pending_events(&mut runtime);
        let baseline = collected_used_heap(&mut runtime);
        for _ in 0..1024 {
            dispatch_native_events(&mut runtime, &events).unwrap();
            flush_pending_events(&mut runtime);
        }
        let after = collected_used_heap(&mut runtime);
        let count = runtime
            .eval::<usize>("globalThis.__event_payload_count")
            .unwrap();
        let growth = after.saturating_sub(baseline);
        let limit = 16_384;
        println!(
            "compat v1 Events V8 heap baseline={baseline} after={after} growth={growth} limit={limit}"
        );
        assert_eq!(count, 5 * 1025, "every event reaches its listener once");
        assert!(
            growth <= limit,
            "collected native-event heap grew {growth} bytes; limit {limit}"
        );
        clear_all();
    }
}
