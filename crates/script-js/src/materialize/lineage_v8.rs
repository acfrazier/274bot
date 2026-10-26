//! Per-isolate CPED lineage carrier and Rust-to-JavaScript entry guards.

use crate::callback_v8::{self, Throw};
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
    interact_gameplay_count: Cell<usize>,
    interact_walk_release_ids: RefCell<Vec<v8::Global<v8::Value>>>,
    interact_push_active: Cell<bool>,
    interact_overflow_dropped: Cell<usize>,
    interact_op_key: v8::Global<v8::String>,
    interact_request_id_key: v8::Global<v8::String>,
    interact_descriptor_value_key: v8::Global<v8::String>,
    interact_descriptor_enumerable_key: v8::Global<v8::String>,
    interact_abort_walk: v8::Global<v8::String>,
    interact_length_key: v8::Global<v8::String>,
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
        let op_key = v8::String::new(&mut scope, "op")
            .ok_or_else(|| "lineage interact op key".to_string())?;
        let request_id_key = v8::String::new(&mut scope, "request_id")
            .ok_or_else(|| "lineage interact request id key".to_string())?;
        let descriptor_value_key = v8::String::new(&mut scope, "value")
            .ok_or_else(|| "lineage interact descriptor key".to_string())?;
        let descriptor_enumerable_key = v8::String::new(&mut scope, "enumerable")
            .ok_or_else(|| "lineage interact descriptor enumerable key".to_string())?;
        let abort_walk = v8::String::new(&mut scope, "abort-walk")
            .ok_or_else(|| "lineage interact abort-walk op".to_string())?;
        let length_key = v8::String::new(&mut scope, "length")
            .ok_or_else(|| "lineage interact length key".to_string())?;
        Rc::new(CarrierState {
            key: v8::Global::new(&mut scope, key),
            hooks: RefCell::new(Vec::new()),
            captured_lineages: RefCell::new(Vec::with_capacity(capacity)),
            interact_gate_enabled,
            interact_gameplay_count: Cell::new(0),
            interact_walk_release_ids: RefCell::new(Vec::with_capacity(if interact_gate_enabled {
                api::script_contract::MAX_WALK_RELEASES
            } else {
                0
            })),
            interact_push_active: Cell::new(false),
            interact_overflow_dropped: Cell::new(0),
            interact_op_key: v8::Global::new(&mut scope, op_key),
            interact_request_id_key: v8::Global::new(&mut scope, request_id_key),
            interact_descriptor_value_key: v8::Global::new(&mut scope, descriptor_value_key),
            interact_descriptor_enumerable_key: v8::Global::new(
                &mut scope,
                descriptor_enumerable_key,
            ),
            interact_abort_walk: v8::Global::new(&mut scope, abort_walk),
            active_entries: Cell::new(0),
            interact_length_key: v8::Global::new(&mut scope, length_key),
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
    // A foreign Proxy can run traps when the lineage symbol is inspected.
    if mapping.is_proxy() {
        return Lineage::Foreign;
    }
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
        captured.extend(std::iter::repeat_n(lineage, count));
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

struct InteractPushGuard<'a>(&'a Cell<bool>);

impl<'a> InteractPushGuard<'a> {
    fn enter(active: &'a Cell<bool>) -> Option<Self> {
        if active.replace(true) {
            None
        } else {
            Some(Self(active))
        }
    }
}

impl Drop for InteractPushGuard<'_> {
    fn drop(&mut self) {
        self.0.set(false);
    }
}

fn interaction_data_field<'s>(
    scope: &mut v8::HandleScope<'s>,
    value: v8::Local<'s, v8::Value>,
    key: &v8::Global<v8::String>,
    descriptor_value_key: &v8::Global<v8::String>,
) -> Result<v8::Local<'s, v8::Value>, Throw<'s>> {
    let undefined = v8::undefined(scope);
    if (!value.is_object() && !value.is_function()) || value.is_proxy() {
        return Ok(undefined.into());
    }
    let Some(object) = value.to_object(scope) else {
        return Ok(undefined.into());
    };
    let key = v8::Local::new(scope, key);
    let descriptor_value_key = v8::Local::new(scope, descriptor_value_key);
    let descriptor_value = callback_v8::catching(scope, |scope| {
        let descriptor = object.get_own_property_descriptor(scope, key.into())?;
        if descriptor.is_undefined() {
            return Some(undefined.into());
        }
        let descriptor = descriptor.to_object(scope)?;
        if !descriptor.has_own_property(scope, descriptor_value_key.into())? {
            return Some(undefined.into());
        }
        descriptor.get(scope, descriptor_value_key.into())
    });
    match descriptor_value {
        Ok(value) => Ok(value),
        Err(Throw::Value(_)) => Ok(undefined.into()),
        Err(Throw::Terminated) => Err(Throw::Terminated),
    }
}

/// Read an array slot from its own data descriptor, never via `Get`.
fn array_data_index<'s>(
    scope: &mut v8::HandleScope<'s>,
    array: v8::Local<'s, v8::Array>,
    index: u32,
    descriptor_value_key: &v8::Global<v8::String>,
) -> Result<Option<v8::Local<'s, v8::Value>>, Throw<'s>> {
    let Some(key) = v8::String::new(scope, &index.to_string()) else {
        return Ok(None);
    };
    let value_key = v8::Local::new(scope, descriptor_value_key);
    let result = callback_v8::catching(scope, |scope| {
        let descriptor = array.get_own_property_descriptor(scope, key.into())?;
        if descriptor.is_undefined() {
            return None;
        }
        let descriptor = descriptor.to_object(scope)?;
        if !descriptor.has_own_property(scope, value_key.into())? {
            return None;
        }
        descriptor.get(scope, value_key.into())
    });
    match result {
        Ok(value) => Ok(Some(value)),
        Err(Throw::Value(_)) => Ok(None),
        Err(Throw::Terminated) => Err(Throw::Terminated),
    }
}

// An interact row has a bounded wire shape (walk avoids are the deepest
// valid value). Refuse unexpected deep or broad object graphs, including
// cycles, instead of keeping a mutable reference in the queued row.
const MAX_INTERACT_SNAPSHOT_DEPTH: usize = 8;
const MAX_INTERACT_SNAPSHOT_ITEMS: u32 = 1024;

/// Copy an own-data-only value graph. A Proxy, accessor, or oversized graph
/// refuses the entire row; neither a descriptor read nor a property write
/// calls user code.
fn snapshot_interact_value<'s>(
    scope: &mut v8::HandleScope<'s>,
    value: v8::Local<'s, v8::Value>,
    descriptor_value_key: v8::Local<'s, v8::String>,
    descriptor_enumerable_key: v8::Local<'s, v8::String>,
    depth: usize,
    objects: &mut usize,
) -> Result<Option<v8::Local<'s, v8::Value>>, Throw<'s>> {
    if value.is_symbol() || value.is_function() || value.is_proxy() {
        return Ok(None);
    }
    if !value.is_object() {
        return Ok(Some(value));
    }
    if depth >= MAX_INTERACT_SNAPSHOT_DEPTH || *objects >= MAX_INTERACT_SNAPSHOT_ITEMS as usize {
        return Ok(None);
    }
    *objects += 1;
    let Some(object) = value.to_object(scope) else {
        return Ok(None);
    };
    let array = v8::Local::<v8::Array>::try_from(value).ok();
    if array.is_some_and(|array| array.length() > MAX_INTERACT_SNAPSHOT_ITEMS) {
        return Ok(None);
    }
    let names = match callback_v8::catching(scope, |scope| {
        object.get_own_property_names(
            scope,
            v8::GetPropertyNamesArgs {
                mode: v8::KeyCollectionMode::OwnOnly,
                property_filter: v8::PropertyFilter::ALL_PROPERTIES
                    | v8::PropertyFilter::SKIP_SYMBOLS,
                index_filter: v8::IndexFilter::IncludeIndices,
                key_conversion: v8::KeyConversionMode::ConvertToString,
            },
        )
    }) {
        Ok(names) if names.length() <= MAX_INTERACT_SNAPSHOT_ITEMS => names,
        Ok(_) | Err(Throw::Value(_)) => return Ok(None),
        Err(Throw::Terminated) => return Err(Throw::Terminated),
    };
    let copy: v8::Local<'s, v8::Object> = if let Some(array) = array {
        v8::Array::new(scope, array.length() as i32).into()
    } else {
        let copy = v8::Object::new(scope);
        let null = v8::null(scope);
        match callback_v8::catching(scope, |scope| copy.set_prototype(scope, null.into())) {
            Ok(true) => copy,
            Ok(false) | Err(Throw::Value(_)) => return Ok(None),
            Err(Throw::Terminated) => return Err(Throw::Terminated),
        }
    };
    for index in 0..names.length() {
        let descriptor = callback_v8::catching(scope, |scope| {
            let name = names.get_index(scope, index)?;
            let name = v8::Local::<v8::Name>::try_from(name).ok()?;
            let descriptor = object.get_own_property_descriptor(scope, name)?;
            if descriptor.is_undefined() {
                return None;
            }
            let descriptor = descriptor.to_object(scope)?;
            if !descriptor.has_own_property(scope, descriptor_value_key.into())?
                || !descriptor.has_own_property(scope, descriptor_enumerable_key.into())?
            {
                return None;
            }
            let property_value = descriptor.get(scope, descriptor_value_key.into())?;
            let enumerable = descriptor.get(scope, descriptor_enumerable_key.into())?;
            Some((name, property_value, enumerable.boolean_value(scope)))
        });
        let (name, property_value, enumerable) = match descriptor {
            Ok(descriptor) => descriptor,
            Err(Throw::Value(_)) => return Ok(None),
            Err(Throw::Terminated) => return Err(Throw::Terminated),
        };
        if !enumerable {
            continue;
        }
        let Some(property_value) = snapshot_interact_value(
            scope,
            property_value,
            descriptor_value_key,
            descriptor_enumerable_key,
            depth + 1,
            objects,
        )?
        else {
            return Ok(None);
        };
        match callback_v8::catching(scope, |scope| {
            copy.create_data_property(scope, name, property_value)
        }) {
            Ok(true) => {}
            Ok(false) | Err(Throw::Value(_)) => return Ok(None),
            Err(Throw::Terminated) => return Err(Throw::Terminated),
        }
    }
    Ok(Some(copy.into()))
}

fn snapshot_interact_row<'s>(
    scope: &mut v8::HandleScope<'s>,
    value: v8::Local<'s, v8::Value>,
    descriptor_value_key: &v8::Global<v8::String>,
    descriptor_enumerable_key: &v8::Global<v8::String>,
) -> Result<Option<v8::Local<'s, v8::Object>>, Throw<'s>> {
    if !value.is_object() || value.is_proxy() {
        return Ok(None);
    }
    let descriptor_value_key = v8::Local::new(scope, descriptor_value_key);
    let descriptor_enumerable_key = v8::Local::new(scope, descriptor_enumerable_key);
    let mut objects = 0;
    let Some(copy) = snapshot_interact_value(
        scope,
        value,
        descriptor_value_key,
        descriptor_enumerable_key,
        0,
        &mut objects,
    )?
    else {
        return Ok(None);
    };
    Ok(v8::Local::<v8::Object>::try_from(copy).ok())
}

fn rollback_interact_capture(state: &CarrierState, captured: usize, dropped: usize) {
    state.captured_lineages.borrow_mut().truncate(captured);
    state.interact_overflow_dropped.set(dropped);
}

fn rollback_interact_rows<'s>(
    scope: &mut v8::HandleScope<'s>,
    state: &CarrierState,
    rows: v8::Local<'s, v8::Array>,
    original_len: u32,
) {
    let length_key = v8::Local::new(scope, &state.interact_length_key);
    let length = v8::Number::new(scope, f64::from(original_len));
    let _ = rows.set(scope, length_key.into(), length.into());
}

fn push_interact_rows<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
) -> Result<v8::Local<'s, v8::Value>, Throw<'s>> {
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
    let Some(_guard) = InteractPushGuard::enter(&state.interact_push_active) else {
        return Err(callback_v8::type_error(
            scope,
            "interact queue append is not reentrant",
        ));
    };
    let Ok(rows) = v8::Local::<v8::Array>::try_from(args.get(0)) else {
        return Err(callback_v8::type_error(scope, "invalid interact queue"));
    };
    if rows.is_proxy() {
        return Err(callback_v8::type_error(scope, "invalid interact queue"));
    }
    let Ok(items) = v8::Local::<v8::Array>::try_from(args.get(1)) else {
        return Err(callback_v8::type_error(
            scope,
            "invalid interact append items",
        ));
    };
    if items.is_proxy() {
        return Err(callback_v8::type_error(
            scope,
            "invalid interact append items",
        ));
    }
    let item_count = items.length();
    let captured_count = state.captured_lineages.borrow().len();
    if !state.interact_gate_enabled
        && captured_count
            .checked_add(item_count as usize)
            .is_none_or(|count| count > api::script_contract::MAX_INTERACT_REQS)
    {
        return Err(callback_v8::type_error(
            scope,
            "interact lineage capacity exceeded",
        ));
    }

    let mut accepted_rows: [Option<v8::Global<v8::Object>>; MAX_CAPTURED_INTERACT_ROWS] =
        std::array::from_fn(|_| None);
    let mut accepted_count = 0;
    let mut dropped = 0;
    let mut gameplay_after = state.interact_gameplay_count.get();
    let existing_release_ids = state.interact_walk_release_ids.borrow();
    let mut new_release_ids: [Option<v8::Global<v8::Value>>;
        api::script_contract::MAX_WALK_RELEASES] = std::array::from_fn(|_| None);
    let mut new_release_count = 0;
    let descriptor_value_key = &state.interact_descriptor_value_key;
    let descriptor_enumerable_key = &state.interact_descriptor_enumerable_key;
    let op_abort_walk = v8::Local::new(scope, &state.interact_abort_walk);

    for index in 0..item_count {
        let Some(item) = array_data_index(scope, items, index, descriptor_value_key)? else {
            dropped += 1;
            continue;
        };
        let Some(item) =
            snapshot_interact_row(scope, item, descriptor_value_key, descriptor_enumerable_key)?
        else {
            dropped += 1;
            continue;
        };
        let is_release = if state.interact_gate_enabled {
            let op = interaction_data_field(
                scope,
                item.into(),
                &state.interact_op_key,
                descriptor_value_key,
            )?;
            op.strict_equals(op_abort_walk.into())
        } else {
            false
        };
        let keep = if is_release {
            let id = interaction_data_field(
                scope,
                item.into(),
                &state.interact_request_id_key,
                descriptor_value_key,
            )?;
            let duplicate = existing_release_ids
                .iter()
                .any(|existing| v8::Local::new(scope, existing).strict_equals(id))
                || new_release_ids[..new_release_count]
                    .iter()
                    .flatten()
                    .any(|existing| v8::Local::new(scope, existing).strict_equals(id));
            if duplicate {
                continue;
            }
            if existing_release_ids.len() + new_release_count
                < api::script_contract::MAX_WALK_RELEASES
            {
                new_release_ids[new_release_count] = Some(v8::Global::new(scope, id));
                new_release_count += 1;
                true
            } else {
                false
            }
        } else if !state.interact_gate_enabled
            || gameplay_after < api::script_contract::MAX_INTERACT_REQS
        {
            if state.interact_gate_enabled {
                gameplay_after += 1;
            }
            true
        } else {
            false
        };
        if keep && accepted_count < accepted_rows.len() {
            accepted_rows[accepted_count] = Some(v8::Global::new(scope, item));
            accepted_count += 1;
        } else {
            dropped += 1;
        }
    }
    drop(existing_release_ids);

    let original_len = rows.length();
    let Ok(accepted_count_u32) = u32::try_from(accepted_count) else {
        return Err(callback_v8::type_error(
            scope,
            "interact queue capacity exceeded",
        ));
    };
    let Some(new_len) = original_len.checked_add(accepted_count_u32) else {
        return Err(callback_v8::type_error(
            scope,
            "interact queue capacity exceeded",
        ));
    };
    let overflow_before = state.interact_overflow_dropped.get();
    capture_interact_lineage(scope, accepted_count, dropped)?;

    let append_result = callback_v8::catching(scope, |scope| {
        for (offset, row) in accepted_rows[..accepted_count].iter().enumerate() {
            let index = original_len + offset as u32;
            // Define an own array index to bypass inherited numeric setters.
            let key = v8::Integer::new_from_unsigned(scope, index).to_string(scope)?;
            let row = v8::Local::new(scope, row.as_ref()?);
            if !rows.create_data_property(scope, key.into(), row.into())? {
                return Some(false);
            }
        }
        Some(true)
    });
    match append_result {
        Ok(true) => {}
        Ok(false) => {
            rollback_interact_rows(scope, &state, rows, original_len);
            rollback_interact_capture(&state, captured_count, overflow_before);
            return Err(callback_v8::type_error(
                scope,
                "interact queue append failed",
            ));
        }
        Err(error) => {
            rollback_interact_rows(scope, &state, rows, original_len);
            rollback_interact_capture(&state, captured_count, overflow_before);
            return Err(error);
        }
    }

    state.interact_gameplay_count.set(gameplay_after);
    let mut release_ids = state.interact_walk_release_ids.borrow_mut();
    for id in new_release_ids[..new_release_count].iter_mut() {
        release_ids.push(id.take().expect("staged walk release id"));
    }
    Ok(v8::Number::new(scope, f64::from(new_len)).into())
}

fn push_interact_rows_callback<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = push_interact_rows(scope, args);
    callback_v8::finish(scope, rv, result);
}

/// Install the private native append used by the host's bounded interact queue.
pub(crate) fn install_interact_capture(runtime: &mut Runtime) -> Result<(), String> {
    callback_v8::install(
        runtime,
        "__rs2b0t_push_interact_rows",
        push_interact_rows_callback,
    )
}

fn reset_interact_admission(state: &CarrierState) {
    state.interact_gameplay_count.set(0);
    state.interact_walk_release_ids.borrow_mut().clear();
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
    reset_interact_admission(&state);
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
        reset_interact_admission(&state);
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
    if !mapping.set_prototype(scope, null.into())? {
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
            if !mapping.set(scope, name, value)? {
                return None;
            }
        }
    }
    let token = v8::BigInt::new_from_u64(scope, handle);
    if !mapping.set(scope, key.into(), token.into())? {
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

/// Module resolution, compilation and evaluation get their own
/// `SETUP_TIMEOUT` (10 s) timer, started when the load starts rather than from
/// the setup owner's remaining deadline. The timer bounds a load that waits on
/// a pending host async operation: deno_core rejects a top-level await that
/// never settles before the timer can fire, and a pending operation sees
/// termination only when this timer next wakes this future. The 50 ms runtime
/// budget still bounds a yielding tick, and the setup owner still interrupts
/// non-yielding V8 execution.
pub(crate) fn load_modules(
    runtime: &mut Runtime,
    main: &rustyscript::Module,
    side: Vec<&rustyscript::Module>,
) -> Result<(), Error> {
    enter_root(runtime, |runtime| {
        let tokio = runtime.tokio_runtime();
        let heap_exhausted = runtime.heap_exhausted_token();
        tokio.block_on(async {
            tokio::select! {
                result = tokio::time::timeout(super::isolate::SETUP_TIMEOUT, async {
                    let loaded = runtime.load_modules_async(main, side).await;
                    runtime
                        .await_event_loop(rustyscript::deno_core::PollEventLoopOptions::default(), None)
                        .await?;
                    loaded.map(|_| ())
                }) => result.map_err(|error| Error::Timeout(error.to_string()))?,
                () = heap_exhausted.cancelled() => Err(Error::HeapExhausted),
            }
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustyscript::RuntimeOptions;

    fn gated_runtime() -> Runtime {
        crate::platform::bootstrap_test_v8_platform();
        let mut runtime = Runtime::new(RuntimeOptions::default()).unwrap();
        install(&mut runtime, true).unwrap();
        enter_root(&mut runtime, install_interact_capture).unwrap();
        runtime
    }
    #[test]
    fn finite_async_side_modules_do_not_share_the_tick_timeout() {
        crate::platform::bootstrap_test_v8_platform();
        let mut runtime = Runtime::new(RuntimeOptions {
            timeout: Duration::from_millis(50),
            ..Default::default()
        })
        .unwrap();
        install(&mut runtime, false).unwrap();
        runtime
            .register_async_function(
                "finish_side",
                rustyscript::async_callback!(|_arg: i32| async move {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                    Ok::<i32, Error>(1)
                }),
            )
            .unwrap();
        let sides: Vec<_> = (0..4)
            .map(|n| {
                rustyscript::Module::new(
                    format!("/rs2b0t/bot/scripts/bot/side{n}.js"),
                    "await rustyscript.async_functions.finish_side(0);",
                )
            })
            .collect();
        let main = rustyscript::Module::new(
            "/rs2b0t/bot/scripts/bot/main.js",
            "export const done = true;",
        );
        load_modules(&mut runtime, &main, sides.iter().collect()).unwrap();
    }

    // `rust-interact-gate-bypass` removes the native caps; this direct callback
    // test must fail even though it does not install the JavaScript queue shim.
    #[test]
    fn rust_interact_gate_blocks_shim_bypass_mutant() {
        let mut runtime = gated_runtime();
        let script = format!(
            r#"(() => {{
                const rows = [];
                const items = [];
                for (let i = 0; i < {gameplay}; i++) items.push({{ op: 'walk-to', request_id: i }});
                for (let i = 0; i < {releases}; i++) items.push({{ op: 'abort-walk', request_id: i }});
                const length = globalThis.__rs2b0t_push_interact_rows(rows, items);
                return {{
                    length,
                    gameplay: rows.filter(row => row.op !== 'abort-walk').length,
                    releases: rows.filter(row => row.op === 'abort-walk').length,
                }};
            }})()"#,
            gameplay = api::script_contract::MAX_INTERACT_REQS + 8,
            releases = api::script_contract::MAX_WALK_RELEASES + 4,
        );
        let result: serde_json::Value = eval(&mut runtime, script).unwrap();
        assert_eq!(
            result["length"],
            api::script_contract::MAX_INTERACT_REQS + api::script_contract::MAX_WALK_RELEASES
        );
        assert_eq!(result["gameplay"], api::script_contract::MAX_INTERACT_REQS);
        assert_eq!(result["releases"], api::script_contract::MAX_WALK_RELEASES);
        assert_eq!(
            take_interact_lineages(&mut runtime).unwrap().len(),
            api::script_contract::MAX_INTERACT_REQS + api::script_contract::MAX_WALK_RELEASES
        );
        assert_eq!(
            take_interact_overflow(&mut runtime).unwrap(),
            12,
            "rows above both Rust-owned caps are counted as dropped"
        );
    }
    #[test]
    fn repeated_abort_walk_ids_do_not_consume_release_capacity() {
        let mut runtime = gated_runtime();
        let script = format!(
            r#"(() => {{
                const rows = [];
                const items = [{{ op: 'abort-walk', request_id: 1 }}, {{ op: 'abort-walk', request_id: 1 }}];
                for (let id = 2; id <= {last_allowed}; id++) items.push({{ op: 'abort-walk', request_id: id }});
                const firstLength = globalThis.__rs2b0t_push_interact_rows(rows, items);
                const nextLength = globalThis.__rs2b0t_push_interact_rows(
                    rows,
                    [{{ op: 'abort-walk', request_id: {over_cap} }}],
                );
                return {{
                    firstLength,
                    nextLength,
                    repeated: rows.filter(row => row.request_id === 1).length,
                    unique: new Set(rows.map(row => row.request_id)).size,
                }};
            }})()"#,
            last_allowed = api::script_contract::MAX_WALK_RELEASES,
            over_cap = api::script_contract::MAX_WALK_RELEASES + 1,
        );
        let result: serde_json::Value = eval(&mut runtime, script).unwrap();
        assert_eq!(
            result["firstLength"],
            api::script_contract::MAX_WALK_RELEASES
        );
        assert_eq!(
            result["nextLength"],
            api::script_contract::MAX_WALK_RELEASES
        );
        assert_eq!(result["repeated"], 1);
        assert_eq!(result["unique"], api::script_contract::MAX_WALK_RELEASES);
        assert_eq!(
            take_interact_lineages(&mut runtime).unwrap().len(),
            api::script_contract::MAX_WALK_RELEASES
        );
        assert_eq!(take_interact_overflow(&mut runtime).unwrap(), 1);
    }

    #[test]
    fn interact_append_refuses_hostile_rows_without_running_user_code() {
        let mut runtime = gated_runtime();
        let result: serde_json::Value = eval(
            &mut runtime,
            r#"(() => {
                const rows = [];
                let inherited = 0;
                let own = 0;
                let proxy = 0;
                let throwing = 0;
                let reentrant = 'not called';
                const accessor = {};
                Object.defineProperty(accessor, 'op', {
                    enumerable: true,
                    get() { own += 1; return 'walk-to'; },
                });
                const accessorId = { op: 'abort-walk' };
                let ownId = 0;
                Object.defineProperty(accessorId, 'request_id', {
                    enumerable: true,
                    get() { ownId += 1; return 1; },
                });
                const proxied = new Proxy({ op: 'walk-to' }, {
                    getOwnPropertyDescriptor(target, key) {
                        proxy += 1;
                        try {
                            globalThis.__rs2b0t_push_interact_rows(rows, [{ op: 'walk-to' }]);
                            reentrant = 'accepted';
                        } catch (error) {
                            reentrant = String(error && error.message);
                        }
                        return Reflect.getOwnPropertyDescriptor(target, key);
                    },
                });
                const throwingItem = new Proxy({ op: 'walk-to' }, {
                    getOwnPropertyDescriptor() {
                        throwing += 1;
                        throw new Error('descriptor trap');
                    },
                });
                Object.defineProperty(Object.prototype, 'value', {
                    get() { inherited += 1; return 'abort-walk'; },
                    configurable: true,
                });
                globalThis.__rs2b0t_push_interact_rows(rows, [
                    accessor,
                    accessorId,
                    proxied,
                    throwingItem,
                ]);
                delete Object.prototype.value;
                return { inherited, own, ownId, proxy, throwing, reentrant, length: rows.length };
            })()"#,
        )
        .unwrap();
        assert_eq!(
            result["inherited"], 0,
            "inherited descriptor getter ran: {result}"
        );
        assert_eq!(result["own"], 0, "own op accessor ran: {result}");
        assert_eq!(result["ownId"], 0, "own request_id accessor ran: {result}");
        assert_eq!(result["proxy"], 0, "Proxy trap ran: {result}");
        assert_eq!(result["throwing"], 0, "throwing Proxy trap ran: {result}");
        assert_eq!(result["reentrant"], "not called");
        assert_eq!(result["length"], 0);
        assert!(take_interact_lineages(&mut runtime).unwrap().is_empty());
        assert_eq!(take_interact_overflow(&mut runtime).unwrap(), 4);
    }

    #[test]
    fn interact_append_copies_rows_before_the_caller_can_mutate_them() {
        let mut runtime = gated_runtime();
        let result: serde_json::Value = eval(
            &mut runtime,
            r#"(() => {
                const rows = [];
                const item = {
                    op: 'walk-to',
                    x: 1,
                    z: 2,
                    level: 0,
                    request_id: 7,
                    note: 'before',
                    avoid: [{ minX: 10, maxX: 12 }],
                    cross: ['safe'],
                };
                globalThis.__rs2b0t_push_interact_rows(rows, [item]);
                item.op = 'abort-walk';
                item.x = 99;
                item.request_id = 99;
                item.note = 'after';
                item.avoid[0].minX = 99;
                item.cross[0] = 'unsafe';
                item.avoid.push({ minX: 100, maxX: 200 });
                return rows[0];
            })()"#,
        )
        .unwrap();
        assert_eq!(result["op"], "walk-to");
        assert_eq!(result["x"], 1);
        assert_eq!(result["request_id"], 7);
        assert_eq!(result["note"], "before");
        assert_eq!(
            result["avoid"],
            serde_json::json!([{ "minX": 10, "maxX": 12 }])
        );
        assert_eq!(result["cross"], serde_json::json!(["safe"]));
        assert_eq!(take_interact_lineages(&mut runtime).unwrap().len(), 1);
        assert_eq!(take_interact_overflow(&mut runtime).unwrap(), 0);
    }

    #[test]
    fn interact_reentrant_push_is_refused_before_release_ids_are_mutated() {
        let mut runtime = gated_runtime();
        let state = state_for_runtime(&mut runtime).unwrap();
        // Model the outer push's immutable release-id borrow during re-entry.
        let release_ids = state.interact_walk_release_ids.borrow();
        state.interact_push_active.set(true);
        let result: String = eval(
            &mut runtime,
            r#"(() => {
                const rows = [];
                try {
                    globalThis.__rs2b0t_push_interact_rows(rows, [
                        { op: 'abort-walk', request_id: 1 },
                    ]);
                    return 'accepted';
                } catch (error) {
                    return String(error && error.message);
                }
            })()"#,
        )
        .unwrap();
        state.interact_push_active.set(false);
        drop(release_ids);
        assert_eq!(result, "interact queue append is not reentrant");
        assert!(take_interact_lineages(&mut runtime).unwrap().is_empty());
        assert_eq!(take_interact_overflow(&mut runtime).unwrap(), 0);
    }

    #[test]
    fn duplicate_releases_do_not_consume_capture_capacity() {
        let mut runtime = gated_runtime();
        let script = format!(
            r#"(() => {{
                const rows = [];
                const gameplay = [];
                for (let i = 0; i < {gameplay}; i++) gameplay.push({{ op: 'walk-to', request_id: i }});
                globalThis.__rs2b0t_push_interact_rows(rows, gameplay);
                for (let i = 0; i < 18; i++) {{
                    globalThis.__rs2b0t_push_interact_rows(rows, [{{ op: 'abort-walk', request_id: 1 }}]);
                }}
                globalThis.__rs2b0t_push_interact_rows(rows, [{{ op: 'abort-walk', request_id: 2 }}]);
                return {{
                    total: rows.length,
                    releases: rows.filter(row => row.op === 'abort-walk').length,
                    unique2: rows.filter(row => row.op === 'abort-walk' && row.request_id === 2).length,
                }};
            }})()"#,
            gameplay = api::script_contract::MAX_INTERACT_REQS,
        );
        let result: serde_json::Value = eval(&mut runtime, script).unwrap();
        assert_eq!(result["total"], api::script_contract::MAX_INTERACT_REQS + 2);
        assert_eq!(result["releases"], 2);
        assert_eq!(result["unique2"], 1);
        assert_eq!(
            take_interact_lineages(&mut runtime).unwrap().len(),
            api::script_contract::MAX_INTERACT_REQS + 2
        );
        assert_eq!(take_interact_overflow(&mut runtime).unwrap(), 0);
    }

    #[test]
    fn interact_append_refuses_accessors_on_items_array_indices() {
        let mut runtime = gated_runtime();
        let result: serde_json::Value = eval(
            &mut runtime,
            r#"(() => {
                const rows = [];
                const items = [];
                let calls = 0;
                let inherited = 0;
                Object.defineProperty(Array.prototype, '1', {
                    get() { inherited += 1; return { op: 'walk-to' }; },
                    configurable: true,
                });
                Object.defineProperty(items, '0', {
                    enumerable: true,
                    get() { calls += 1; return { op: 'walk-to' }; },
                });
                items.length = 2;
                globalThis.__rs2b0t_push_interact_rows(rows, items);
                delete Array.prototype[1];
                return { calls, inherited, length: rows.length };
            })()"#,
        )
        .unwrap();
        assert_eq!(result["calls"], 0, "items accessor ran: {result}");
        assert_eq!(result["inherited"], 0, "hole getter ran: {result}");
        assert_eq!(result["length"], 0);
        assert!(take_interact_lineages(&mut runtime).unwrap().is_empty());
        assert_eq!(take_interact_overflow(&mut runtime).unwrap(), 2);
    }

    #[test]
    fn interact_append_does_not_call_inherited_array_index_setters() {
        let mut runtime = gated_runtime();
        let result: serde_json::Value = eval(
            &mut runtime,
            r#"(() => {
                const rows = [];
                const items = [];
                let setterCalls = 0;
                Object.defineProperty(Array.prototype, '0', {
                    set() { setterCalls += 1; },
                    configurable: true,
                });
                Object.defineProperty(items, '0', {
                    value: { op: 'walk-to', x: 1, z: 2, level: 0 },
                    enumerable: true,
                    configurable: true,
                });
                items.length = 1;
                globalThis.__rs2b0t_push_interact_rows(rows, items);
                delete Array.prototype[0];
                return { setterCalls, length: rows.length, op: rows[0].op };
            })()"#,
        )
        .unwrap();
        assert_eq!(result["setterCalls"], 0);
        assert_eq!(result["length"], 1);
        assert_eq!(result["op"], "walk-to");
        assert_eq!(take_interact_lineages(&mut runtime).unwrap().len(), 1);
        assert_eq!(take_interact_overflow(&mut runtime).unwrap(), 0);
    }

    /// A Proxy nested inside a row (here a walk `avoid` entry) refuses the
    /// whole row without running a trap, and the refusal is counted. A plain
    /// nested value in the same push is still admitted.
    #[test]
    fn interact_append_refuses_a_proxy_nested_inside_a_row() {
        let mut runtime = gated_runtime();
        let result: serde_json::Value = eval(
            &mut runtime,
            r#"(() => {
                const rows = [];
                let traps = 0;
                const nested = new Proxy({ minX: 1, maxX: 2 }, {
                    ownKeys(target) { traps += 1; return Reflect.ownKeys(target); },
                    getOwnPropertyDescriptor(target, key) {
                        traps += 1;
                        return Reflect.getOwnPropertyDescriptor(target, key);
                    },
                    get(target, key) { traps += 1; return Reflect.get(target, key); },
                });
                globalThis.__rs2b0t_push_interact_rows(rows, [
                    { op: 'walk-to', x: 1, z: 1, level: 0, avoid: [nested] },
                    { op: 'walk-to', x: 2, z: 2, level: 0, avoid: [{ minX: 1, maxX: 2 }] },
                ]);
                return { traps, total: rows.length, x: rows.length === 1 ? rows[0].x : null };
            })()"#,
        )
        .unwrap();
        assert_eq!(result["traps"], 0, "a nested Proxy trap ran: {result}");
        assert_eq!(
            result["total"], 1,
            "the nested Proxy row is refused: {result}"
        );
        assert_eq!(result["x"], 2, "the plain row is kept: {result}");
        assert_eq!(take_interact_lineages(&mut runtime).unwrap().len(), 1);
        assert_eq!(take_interact_overflow(&mut runtime).unwrap(), 1);
    }
}
