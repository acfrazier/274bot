//! Caller-callback invocation for typed V8 helpers.
//!
//! Operator decision Q1 (2026-09-23, `FENCE.md`): Rust invokes the catalog
//! script's own JS callbacks directly. A frozen rs2b0t helper such as
//! `bestPickaxe(level, available)` or `potionToSip({plans, held, levels})`
//! becomes one native call: the shim passes the caller's arguments unchanged,
//! and the Rust helper walks the loop, calling the caller's functions in the
//! frozen order with the frozen arguments and receiver. This module is the one
//! mechanism every such helper uses; it is local V8 marshalling inside the
//! isolate, never a host transport (the FlatBuffer stays the only
//! isolate/host wire, and nothing here sends a game action).
//!
//! # Contract
//!
//! - Every JS-observable step — a property read (getters run), a call, a
//!   `ToNumber`/`ToString` conversion, an iterator step — goes through
//!   [`catching`], so a throw becomes [`Throw::Value`] carrying the thrown value
//!   itself. [`finish`] rethrows that value at the helper boundary, so the
//!   script's `catch` sees exactly what its callback threw (same object, same
//!   stack).
//! - A watchdog termination is [`Throw::Terminated`]: it is rethrown inside
//!   the `TryCatch` that saw it and nothing else runs.
//! - Errors the frozen body itself would raise (reading a property of
//!   `undefined`, calling a non-function, iterating a non-iterable) are
//!   `TypeError`s with the engine's message shape ([`type_error`]).
//! - [`ForOf`] is the frozen `for...of`: the caller's iterator is acquired
//!   once, `next` is read once, a returning hit closes it with a normal
//!   completion and a throwing body closes it with a throw completion.
//! - Truthiness is JS `ToBoolean` ([`truthy`]); relational operators follow
//!   JS: two strings compare by UTF-16 code units, anything else by
//!   `ToNumber` ([`compare`]). BigInt operands and objects whose
//!   `Symbol.toPrimitive` yields a string are outside the typed callers'
//!   domain and take the numeric path.
//!
//! # Extension point: callbacks held across ticks (F03)
//!
//! [`Callback`] is borrowed for one native call: its `Local` handles die with
//! the call's `HandleScope`. A Rust step machine that must keep a caller hook
//! across ticks promotes it with `v8::Global::new(scope, func)` (and the
//! receiver likewise), stores the `Global` in its machine state, and re-opens
//! it as a `Local` inside the tick's scope before calling. An async hook's
//! return value is checked with `value.is_promise()`; the machine then keeps
//! the `v8::Global<v8::Promise>` and polls `Promise::state()` once per tick
//! (`Pending` → wait, `Fulfilled` → `result()`, `Rejected` → rethrow
//! `result()` through [`finish`]'s rethrow path). Only the synchronous path is
//! implemented here because none of the helpers that use it today await.

use rustyscript::Runtime;
use std::cmp::Ordering;

/// Why a JS-observable step did not produce a value.
pub(crate) enum Throw<'s> {
    /// The value a callback, getter or conversion threw (or a frozen
    /// `TypeError` this helper raises); rethrown unchanged by [`finish`].
    Value(v8::Local<'s, v8::Value>),
    /// Execution was terminated; already rethrown, nothing else may run.
    Terminated,
}

pub(crate) type JsResult<'s, T> = Result<T, Throw<'s>>;

/// Install `callback` as a global function named `name`.
pub(crate) fn install(
    runtime: &mut Runtime,
    name: &str,
    callback: impl v8::MapFnTo<v8::FunctionCallback>,
) -> Result<(), String> {
    let context = runtime.deno_runtime().main_context();
    let mut scope = runtime.deno_runtime().handle_scope();
    let global = context.open(&mut scope).global(&mut scope);
    let key = v8::String::new(&mut scope, name).ok_or_else(|| format!("{name}: name"))?;
    let func = v8::Function::new(&mut scope, callback).ok_or_else(|| format!("{name}: fn"))?;
    global
        .set(&mut scope, key.into(), func.into())
        .ok_or_else(|| format!("{name}: set"))?;
    Ok(())
}

/// The helper boundary: return the value, or rethrow what was thrown.
pub(crate) fn finish<'s>(
    scope: &mut v8::HandleScope<'s>,
    mut rv: v8::ReturnValue,
    result: JsResult<'s, v8::Local<'s, v8::Value>>,
) {
    match result {
        Ok(value) => rv.set(value),
        Err(Throw::Value(exception)) => {
            scope.throw_exception(exception);
        }
        Err(Throw::Terminated) => {}
    }
}

/// Run one JS-observable V8 operation, turning a throw into [`Throw`].
pub(crate) fn catching<'s, T>(
    scope: &mut v8::HandleScope<'s>,
    f: impl FnOnce(&mut v8::HandleScope<'s>) -> Option<T>,
) -> JsResult<'s, T> {
    let tc = &mut v8::TryCatch::new(scope);
    if let Some(value) = f(tc) {
        return Ok(value);
    }
    if tc.has_terminated() || !tc.can_continue() {
        tc.rethrow();
        return Err(Throw::Terminated);
    }
    match tc.exception() {
        Some(exception) => Err(Throw::Value(exception)),
        None => {
            let message = v8::String::new(tc, "callback helper: empty V8 result")
                .unwrap_or_else(|| v8::String::empty(tc));
            Err(Throw::Value(v8::Exception::error(tc, message)))
        }
    }
}

/// A `TypeError` the frozen body itself would raise.
pub(crate) fn type_error<'s>(scope: &mut v8::HandleScope<'s>, message: &str) -> Throw<'s> {
    let message = v8::String::new(scope, message).unwrap_or_else(|| v8::String::empty(scope));
    Throw::Value(v8::Exception::type_error(scope, message))
}

/// The shim's explicit `notImpl(feature)` error (`not impl: <feature>`).
pub(crate) fn not_impl<'s>(scope: &mut v8::HandleScope<'s>, feature: &str) -> Throw<'s> {
    let text = format!("not impl: {feature}");
    let message = v8::String::new(scope, &text).unwrap_or_else(|| v8::String::empty(scope));
    Throw::Value(v8::Exception::error(scope, message))
}

fn nullish_name(value: v8::Local<v8::Value>) -> &'static str {
    if value.is_null() {
        "null"
    } else {
        "undefined"
    }
}

/// `target[key]`: getters run; `undefined`/`null` targets throw the engine's
/// `Cannot read properties of …` `TypeError`.
pub(crate) fn get<'s>(
    scope: &mut v8::HandleScope<'s>,
    target: v8::Local<'s, v8::Value>,
    key: &str,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    if target.is_null_or_undefined() {
        let message = format!(
            "Cannot read properties of {} (reading '{key}')",
            nullish_name(target)
        );
        return Err(type_error(scope, &message));
    }
    let name = string(scope, key);
    catching(scope, |s| target.to_object(s)?.get(s, name))
}

/// `target[index]`.
pub(crate) fn get_index<'s>(
    scope: &mut v8::HandleScope<'s>,
    target: v8::Local<'s, v8::Value>,
    index: u32,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    if target.is_null_or_undefined() {
        let message = format!(
            "Cannot read properties of {} (reading '{index}')",
            nullish_name(target)
        );
        return Err(type_error(scope, &message));
    }
    catching(scope, |s| target.to_object(s)?.get_index(s, index))
}

/// `func.call(recv, ...args)`. A non-callable `func` is the frozen call site's
/// `<label> is not a function` `TypeError`, raised only when the call is reached.
pub(crate) fn call<'s>(
    scope: &mut v8::HandleScope<'s>,
    func: v8::Local<'s, v8::Value>,
    recv: v8::Local<'s, v8::Value>,
    args: &[v8::Local<'s, v8::Value>],
    label: &str,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let Ok(func) = v8::Local::<v8::Function>::try_from(func) else {
        return Err(type_error(scope, &format!("{label} is not a function")));
    };
    catching(scope, |s| func.call(s, recv, args))
}

/// `target.key(...args)`: one property read, then a call with `this = target`.
pub(crate) fn call_method<'s>(
    scope: &mut v8::HandleScope<'s>,
    target: v8::Local<'s, v8::Value>,
    key: &str,
    args: &[v8::Local<'s, v8::Value>],
    label: &str,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let func = get(scope, target, key)?;
    call(scope, func, target, args, label)
}

/// `new ctor(...args)`.
pub(crate) fn construct<'s>(
    scope: &mut v8::HandleScope<'s>,
    ctor: v8::Local<'s, v8::Value>,
    args: &[v8::Local<'s, v8::Value>],
    label: &str,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let Ok(ctor) = v8::Local::<v8::Function>::try_from(ctor) else {
        return Err(type_error(scope, &format!("{label} is not a constructor")));
    };
    catching(scope, |s| ctor.new_instance(s, args).map(Into::into))
}

/// A caller-supplied function and the receiver the frozen call site uses.
///
/// Typed only by where it came from: calling a non-function throws the frozen
/// `TypeError` at the call, not when the helper starts (a frozen helper that
/// never reaches the call never notices).
#[derive(Clone, Copy)]
pub(crate) struct Callback<'s> {
    func: v8::Local<'s, v8::Value>,
    recv: v8::Local<'s, v8::Value>,
    label: &'static str,
}

impl<'s> Callback<'s> {
    /// `label(...)` called as a plain function (`this` is `undefined`).
    pub(crate) fn plain(
        scope: &mut v8::HandleScope<'s>,
        func: v8::Local<'s, v8::Value>,
        label: &'static str,
    ) -> Self {
        Self {
            func,
            recv: v8::undefined(scope).into(),
            label,
        }
    }

    /// `recv.label(...)`, with the method already read from `recv`.
    pub(crate) fn method(
        func: v8::Local<'s, v8::Value>,
        recv: v8::Local<'s, v8::Value>,
        label: &'static str,
    ) -> Self {
        Self { func, recv, label }
    }

    pub(crate) fn call(
        &self,
        scope: &mut v8::HandleScope<'s>,
        args: &[v8::Local<'s, v8::Value>],
    ) -> JsResult<'s, v8::Local<'s, v8::Value>> {
        call(scope, self.func, self.recv, args, self.label)
    }

    /// The call's JS truthiness (`if (cb(x))`, `!!cb(x)`, `a && cb(x)`).
    pub(crate) fn truthy(
        &self,
        scope: &mut v8::HandleScope<'s>,
        args: &[v8::Local<'s, v8::Value>],
    ) -> JsResult<'s, bool> {
        let value = self.call(scope, args)?;
        Ok(truthy(scope, value))
    }
}

/// JS `ToBoolean` (never throws).
pub(crate) fn truthy(scope: &mut v8::HandleScope, value: v8::Local<v8::Value>) -> bool {
    value.boolean_value(scope)
}

/// JS `ToNumber` (runs `valueOf`/`toString`).
pub(crate) fn number<'s>(
    scope: &mut v8::HandleScope<'s>,
    value: v8::Local<'s, v8::Value>,
) -> JsResult<'s, f64> {
    if value.is_number() {
        return Ok(value.number_value(scope).unwrap_or(f64::NAN));
    }
    catching(scope, |s| value.number_value(s))
}

/// JS `ToString`.
pub(crate) fn to_string<'s>(
    scope: &mut v8::HandleScope<'s>,
    value: v8::Local<'s, v8::Value>,
) -> JsResult<'s, String> {
    let s = catching(scope, |s| value.to_string(s))?;
    Ok(s.to_rust_string_lossy(scope))
}

/// JS abstract relational comparison of `left` and `right`, converting `left`
/// first. `None` is an unordered (`NaN`) comparison.
pub(crate) fn compare<'s>(
    scope: &mut v8::HandleScope<'s>,
    left: v8::Local<'s, v8::Value>,
    right: v8::Local<'s, v8::Value>,
) -> JsResult<'s, Option<Ordering>> {
    if left.is_string() && right.is_string() {
        let l = left.to_rust_string_lossy(scope);
        let r = right.to_rust_string_lossy(scope);
        return Ok(Some(l.encode_utf16().cmp(r.encode_utf16())));
    }
    let l = number(scope, left)?;
    let r = number(scope, right)?;
    Ok(l.partial_cmp(&r))
}

/// `left >= right`.
pub(crate) fn ge<'s>(
    scope: &mut v8::HandleScope<'s>,
    left: v8::Local<'s, v8::Value>,
    right: v8::Local<'s, v8::Value>,
) -> JsResult<'s, bool> {
    Ok(matches!(
        compare(scope, left, right)?,
        Some(Ordering::Greater | Ordering::Equal)
    ))
}

/// `left <= right`.
pub(crate) fn le<'s>(
    scope: &mut v8::HandleScope<'s>,
    left: v8::Local<'s, v8::Value>,
    right: v8::Local<'s, v8::Value>,
) -> JsResult<'s, bool> {
    Ok(matches!(
        compare(scope, left, right)?,
        Some(Ordering::Less | Ordering::Equal)
    ))
}

/// `left > right`.
pub(crate) fn gt<'s>(
    scope: &mut v8::HandleScope<'s>,
    left: v8::Local<'s, v8::Value>,
    right: v8::Local<'s, v8::Value>,
) -> JsResult<'s, bool> {
    Ok(compare(scope, left, right)? == Some(Ordering::Greater))
}

/// `left < right`.
pub(crate) fn lt<'s>(
    scope: &mut v8::HandleScope<'s>,
    left: v8::Local<'s, v8::Value>,
    right: v8::Local<'s, v8::Value>,
) -> JsResult<'s, bool> {
    Ok(compare(scope, left, right)? == Some(Ordering::Less))
}

/// `left - right` (always numeric).
pub(crate) fn sub<'s>(
    scope: &mut v8::HandleScope<'s>,
    left: v8::Local<'s, v8::Value>,
    right: v8::Local<'s, v8::Value>,
) -> JsResult<'s, f64> {
    let l = number(scope, left)?;
    let r = number(scope, right)?;
    Ok(l - r)
}

/// `left + right`: string concatenation when either operand is a string,
/// numeric addition otherwise.
pub(crate) fn add<'s>(
    scope: &mut v8::HandleScope<'s>,
    left: v8::Local<'s, v8::Value>,
    right: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    if left.is_string() || right.is_string() {
        let mut out = to_string(scope, left)?;
        out.push_str(&to_string(scope, right)?);
        return Ok(string(scope, &out));
    }
    let l = number(scope, left)?;
    let r = number(scope, right)?;
    Ok(v8::Number::new(scope, l + r).into())
}

/// A V8 string value (an over-long string degrades to `""`).
pub(crate) fn string<'s>(scope: &mut v8::HandleScope<'s>, s: &str) -> v8::Local<'s, v8::Value> {
    v8::String::new(scope, s)
        .unwrap_or_else(|| v8::String::empty(scope))
        .into()
}

pub(crate) fn num<'s>(scope: &mut v8::HandleScope<'s>, n: f64) -> v8::Local<'s, v8::Value> {
    v8::Number::new(scope, n).into()
}

/// A plain object literal `{ key: value, ... }` in field order.
pub(crate) fn object<'s>(
    scope: &mut v8::HandleScope<'s>,
    fields: &[(&str, v8::Local<'s, v8::Value>)],
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let obj = v8::Object::new(scope);
    for (key, value) in fields {
        let key = string(scope, key);
        catching(scope, |s| obj.create_data_property(s, key.cast(), *value))?;
    }
    Ok(obj.into())
}

/// The frozen `for (const x of iterable)` over the caller's own iterable.
pub(crate) struct ForOf<'s> {
    iterator: v8::Local<'s, v8::Value>,
    next: v8::Local<'s, v8::Value>,
}

impl<'s> ForOf<'s> {
    /// `GetIterator(iterable)`; `label` names the iterable in the frozen
    /// `<label> is not iterable` `TypeError`.
    pub(crate) fn open(
        scope: &mut v8::HandleScope<'s>,
        iterable: v8::Local<'s, v8::Value>,
        label: &str,
    ) -> JsResult<'s, Self> {
        let not_iterable = format!("{label} is not iterable");
        if iterable.is_null_or_undefined() {
            return Err(type_error(scope, &not_iterable));
        }
        let symbol: v8::Local<v8::Value> = v8::Symbol::get_iterator(scope).into();
        let method = catching(scope, |s| iterable.to_object(s)?.get(s, symbol))?;
        if !method.is_function() {
            return Err(type_error(scope, &not_iterable));
        }
        let iterator = call(scope, method, iterable, &[], label)?;
        if !iterator.is_object() {
            return Err(type_error(
                scope,
                "Result of the Symbol.iterator method is not an object",
            ));
        }
        let next = get(scope, iterator, "next")?;
        Ok(Self { iterator, next })
    }

    /// One `IteratorStep`: the next value, or `None` once the iterator is done.
    /// A throwing `next` does not close the iterator (frozen semantics).
    pub(crate) fn step(
        &self,
        scope: &mut v8::HandleScope<'s>,
    ) -> JsResult<'s, Option<v8::Local<'s, v8::Value>>> {
        let result = self.call_method(scope, self.next)?;
        if !result.is_object() {
            let shown = to_string(scope, result).unwrap_or_default();
            return Err(type_error(
                scope,
                &format!("Iterator result {shown} is not an object"),
            ));
        }
        let done = get(scope, result, "done")?;
        if truthy(scope, done) {
            return Ok(None);
        }
        Ok(Some(get(scope, result, "value")?))
    }

    /// Call an iterator protocol method; a non-callable one is the engine's
    /// `<type> <value> is not a function` `TypeError`.
    fn call_method(
        &self,
        scope: &mut v8::HandleScope<'s>,
        method: v8::Local<'s, v8::Value>,
    ) -> JsResult<'s, v8::Local<'s, v8::Value>> {
        if !method.is_function() {
            let kind = method.type_of(scope).to_rust_string_lossy(scope);
            let shown = if method.is_object() {
                String::from("#<Object>")
            } else {
                to_string(scope, method).unwrap_or_default()
            };
            return Err(type_error(scope, &format!("{kind} {shown} is not a function")));
        }
        call(scope, method, self.iterator, &[], "iterator method")
    }

    /// Leaving the loop with a `return`/`break`: `IteratorClose` (normal).
    pub(crate) fn close(&self, scope: &mut v8::HandleScope<'s>) -> JsResult<'s, ()> {
        let ret = get(scope, self.iterator, "return")?;
        if ret.is_null_or_undefined() {
            return Ok(());
        }
        let result = self.call_method(scope, ret)?;
        if !result.is_object() {
            let shown = to_string(scope, result).unwrap_or_default();
            return Err(type_error(
                scope,
                &format!("Iterator result {shown} is not an object"),
            ));
        }
        Ok(())
    }

    /// A loop body's outcome: a throw closes the iterator (`IteratorClose`
    /// with a throw completion: `return` runs, its own outcome is dropped) and
    /// the body's throw is what propagates.
    pub(crate) fn body<T>(
        &self,
        scope: &mut v8::HandleScope<'s>,
        outcome: JsResult<'s, T>,
    ) -> JsResult<'s, T> {
        let thrown = match outcome {
            Ok(value) => return Ok(value),
            Err(Throw::Terminated) => return Err(Throw::Terminated),
            Err(thrown) => thrown,
        };
        let closed = get(scope, self.iterator, "return").and_then(|ret| {
            if ret.is_null_or_undefined() {
                Ok(())
            } else {
                self.call_method(scope, ret).map(|_| ())
            }
        });
        match closed {
            Err(Throw::Terminated) => Err(Throw::Terminated),
            _ => Err(thrown),
        }
    }
}
