//! v1 `chooseTarget` (frozen `bot/api/thieving/targets.ts`) as one native
//! call: `__rs2b0t_choose_target(candidatesNearestFirst, reachable)`.
//!
//! ```text
//! for (const c of candidatesNearestFirst) {
//!     if (reachable(c)) return { target: c, blocked: null };
//! }
//! return { target: null, blocked: candidatesNearestFirst[0] ?? null };
//! ```
//!
//! The caller's own iterable is walked with the frozen `for...of`
//! ([`ForOf`]); `reachable` is called once per visited candidate until the
//! first truthy answer, which closes the iterator.

use super::callback_v8::{self as cb, Callback, Flow, ForOf, JsResult};
use rustyscript::Runtime;

pub(super) fn install(runtime: &mut Runtime) -> Result<(), String> {
    cb::install(runtime, "__rs2b0t_choose_target", choose_target)
}

fn choose_target<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = run(scope, args.get(0), args.get(1));
    cb::finish(scope, rv, result);
}

fn run<'s>(
    scope: &mut v8::HandleScope<'s>,
    candidates: v8::Local<'s, v8::Value>,
    reachable: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let reachable = Callback::plain(scope, reachable, "reachable");
    let null: v8::Local<v8::Value> = v8::null(scope).into();
    let walk = ForOf::open(scope, candidates, "candidatesNearestFirst")?;
    let target = loop {
        let scope = &mut v8::EscapableHandleScope::new(scope);
        let flow = candidate_iteration(scope, &walk, reachable);
        if let Flow::Break(target) = cb::iteration(scope, flow)? {
            break target;
        }
    };
    if let Some(target) = target {
        return cb::object(scope, &[("target", target), ("blocked", null)]);
    }
    let first = cb::get_index(scope, candidates, 0)?;
    let blocked = if first.is_null_or_undefined() { null } else { first };
    cb::object(scope, &[("target", null), ("blocked", blocked)])
}

fn candidate_iteration<'s>(
    scope: &mut v8::HandleScope<'s>,
    walk: &ForOf<'s>,
    reachable: Callback<'s>,
) -> JsResult<'s, Flow<'s>> {
    let Some(candidate) = walk.step(scope)? else {
        return Ok(Flow::Break(None));
    };
    let hit = reachable.truthy(scope, &[candidate]);
    if walk.body(scope, hit)? {
        walk.close(scope)?;
        return Ok(Flow::Break(Some(candidate)));
    }
    Ok(Flow::Continue(None))
}
