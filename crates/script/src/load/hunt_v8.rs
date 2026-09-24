//! `__rs2b0t_hunt(op, family, ...)`: the synchronous half of the hunt
//! Task classes (`crate::hunt`). One native call per class method:
//!
//! - `('begin', family, site)` mints a session token and keeps its site;
//! - `('validate', family, token, hooks)` / `('blocksLoot', 'hunt-fight',
//!   token, hooks)` read the script's getters and `inArea` predicate off
//!   `hooks` through the callback path, then answer from the policy;
//! - `('reset' | 'interruptWatch', 'hunt-fight', token)`.
//!
//! A getter that throws rethrows here unchanged. Nothing is sent to the
//! host: the asynchronous half (`execute`) is one machine run.

use super::callback_v8::{self, Throw};
use crate::hunt::{self, Host, Kind, HOOKS};
use crate::hunt_fight::{FightKind, HoldKind, RetreatKind, WalkSpotKind};
use crate::hunt_lair::Enter;
use crate::machine::Ended;
use rustyscript::deno_core::serde_v8;
use rustyscript::Runtime;
use serde_json::Value;

pub(super) fn install(runtime: &mut Runtime) -> Result<(), String> {
    callback_v8::install(runtime, "__rs2b0t_hunt", hunt_callback)?;
    callback_v8::install(runtime, "__rs2b0t_hunt_logic", logic_callback)?;
    callback_v8::install(runtime, "__rs2b0t_hunt_dose", dose_callback)
}

/// `__rs2b0t_hunt_logic(name, ...args)`: one frozen hunting function
/// (`crate::hunt_catalog::call`) over the caller's plain arguments.
fn logic_callback<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let name = args.get(0).to_rust_string_lossy(scope);
    let mut values = Vec::with_capacity(args.length().max(1) as usize - 1);
    for i in 1..args.length() {
        let arg = args.get(i);
        values.push(if arg.is_undefined() {
            Value::Null
        } else {
            serde_v8::from_v8(scope, arg).unwrap_or(Value::Null)
        });
    }
    let result = match crate::hunt_catalog::call(&name, &values) {
        Ok(value) => match serde_v8::to_v8(scope, &value) {
            Ok(value) => Ok(value),
            Err(e) => Err(callback_v8::type_error(scope, &e.to_string())),
        },
        Err(message) => Err(callback_v8::not_impl(
            scope,
            message.trim_start_matches("not impl: "),
        )),
    };
    callback_v8::finish(scope, rv, result);
}

/// Frozen `doseToDrink(count, doses)`: the smallest held dose form, the
/// caller's `count` asked in the frozen order (last dose form first).
fn dose_callback<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let count = args.get(0);
    let doses: Vec<String> = serde_v8::from_v8(scope, args.get(1)).unwrap_or_default();
    let recv = v8::undefined(scope).into();
    let mut result = Ok(v8::null(scope).into());
    for name in doses.iter().rev() {
        let arg = callback_v8::string(scope, name);
        let held = callback_v8::call(scope, count, recv, &[arg], "count")
            .and_then(|held| callback_v8::number(scope, held));
        match held {
            Ok(held) if held > 0.0 => {
                result = Ok(arg);
                break;
            }
            Ok(_) => {}
            Err(throw) => {
                result = Err(throw);
                break;
            }
        }
    }
    callback_v8::finish(scope, rv, result);
}

fn hunt_callback<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let op = args.get(0).to_rust_string_lossy(scope);
    let family = args.get(1).to_rust_string_lossy(scope);
    let result = match op.as_str() {
        "begin" => begin(scope, &family, args.get(2)),
        "validate" | "blocksLoot" => read(scope, &op, &family, args.get(2), args.get(3)),
        // Frozen `feePrepaid(site)`: the enter session's own paid-for key.
        "feePrepaid" => {
            let token = token(scope, args.get(2));
            let key = args.get(3).to_rust_string_lossy(scope);
            Ok(v8::Boolean::new(scope, crate::hunt_lair::fee_paid_for(token, &key)).into())
        }
        "reset" | "interruptWatch" if family == FightKind::NAME => {
            let token = token(scope, args.get(2));
            if op == "reset" {
                crate::hunt_fight::reset(token);
            } else {
                crate::hunt_fight::interrupt_watch(token);
            }
            Ok(v8::undefined(scope).into())
        }
        _ => Err(callback_v8::not_impl(scope, &format!("hunt {op} {family}"))),
    };
    callback_v8::finish(scope, rv, result);
}

fn token(scope: &mut v8::HandleScope, value: v8::Local<v8::Value>) -> u64 {
    value
        .number_value(scope)
        .filter(|n| n.is_finite() && *n >= 0.0)
        .map_or(0, |n| n as u64)
}

fn begin<'s>(
    scope: &mut v8::HandleScope<'s>,
    family: &str,
    site: v8::Local<'s, v8::Value>,
) -> callback_v8::JsResult<'s, v8::Local<'s, v8::Value>> {
    let site: Value = match serde_v8::from_v8(scope, site) {
        Ok(site @ Value::Object(_)) => site,
        _ => {
            return Err(callback_v8::type_error(
                scope,
                "hunt site must be an object",
            ))
        }
    };
    let token = match family {
        FightKind::NAME => hunt::begin_session::<FightKind>(&site),
        HoldKind::NAME => hunt::begin_session::<HoldKind>(&site),
        RetreatKind::NAME => hunt::begin_session::<RetreatKind>(&site),
        WalkSpotKind::NAME => hunt::begin_session::<WalkSpotKind>(&site),
        Enter::NAME => hunt::begin_session::<Enter>(&site),
        _ => {
            return Err(callback_v8::not_impl(
                scope,
                &format!("hunt begin {family}"),
            ))
        }
    };
    Ok(callback_v8::num(scope, token as f64))
}

fn read<'s>(
    scope: &mut v8::HandleScope<'s>,
    op: &str,
    family: &str,
    token: v8::Local<'s, v8::Value>,
    hooks: v8::Local<'s, v8::Value>,
) -> callback_v8::JsResult<'s, v8::Local<'s, v8::Value>> {
    let token = self::token(scope, token);
    let mut host = V8Host {
        scope,
        hooks,
        held: vec![None; HOOKS.len()],
        thrown: None,
    };
    let answer = match (op, family) {
        ("blocksLoot", FightKind::NAME) => {
            hunt::with_session::<FightKind, _>(token, &mut host, crate::hunt_fight::blocks_loot)
        }
        ("validate", FightKind::NAME) => validate::<FightKind>(token, &mut host),
        ("validate", HoldKind::NAME) => validate::<HoldKind>(token, &mut host),
        ("validate", RetreatKind::NAME) => validate::<RetreatKind>(token, &mut host),
        ("validate", WalkSpotKind::NAME) => validate::<WalkSpotKind>(token, &mut host),
        ("validate", Enter::NAME) => validate::<Enter>(token, &mut host),
        _ => {
            let scope = host.scope;
            return Err(callback_v8::not_impl(scope, &format!("hunt {op} {family}")));
        }
    };
    let V8Host { scope, thrown, .. } = host;
    match answer {
        // A token this isolate never began reads false, as a dead one did.
        None => Ok(v8::Boolean::new(scope, false).into()),
        Some(Ok(value)) => Ok(v8::Boolean::new(scope, value).into()),
        Some(Err(Ended)) => Err(thrown.unwrap_or(Throw::Terminated)),
    }
}

fn validate<K: Kind>(token: u64, host: &mut dyn Host) -> Option<Result<bool, Ended>> {
    hunt::with_session::<K, _>(token, host, K::validate)
}

/// The class's `hooks` object as the hunt host: each hook is read once,
/// when first needed, and called with `this = hooks`.
struct V8Host<'a, 's> {
    scope: &'a mut v8::HandleScope<'s>,
    hooks: v8::Local<'s, v8::Value>,
    held: Vec<Option<v8::Local<'s, v8::Value>>>,
    thrown: Option<Throw<'s>>,
}

impl<'s> V8Host<'_, 's> {
    fn hook(&mut self, hook: usize) -> Result<v8::Local<'s, v8::Value>, Ended> {
        if let Some(func) = self.held.get(hook).copied().flatten() {
            return Ok(func);
        }
        let name = HOOKS.get(hook).copied().unwrap_or("");
        match callback_v8::get(self.scope, self.hooks, name) {
            Ok(func) => {
                if let Some(slot) = self.held.get_mut(hook) {
                    *slot = Some(func);
                }
                Ok(func)
            }
            Err(throw) => {
                self.thrown = Some(throw);
                Err(Ended)
            }
        }
    }
}

impl Host for V8Host<'_, '_> {
    fn has(&mut self, hook: usize) -> bool {
        self.thrown.is_none()
            && self
                .hook(hook)
                .is_ok_and(|func| !func.is_null_or_undefined())
    }

    fn ask(&mut self, hook: usize, args: &[Value]) -> Result<Value, Ended> {
        if self.thrown.is_some() {
            return Err(Ended);
        }
        let func = self.hook(hook)?;
        let name = HOOKS.get(hook).copied().unwrap_or("hook");
        let mut argv = Vec::with_capacity(args.len());
        for arg in args {
            match serde_v8::to_v8(self.scope, arg) {
                Ok(value) => argv.push(value),
                Err(e) => {
                    self.thrown = Some(callback_v8::type_error(self.scope, &e.to_string()));
                    return Err(Ended);
                }
            }
        }
        let value = match callback_v8::call(self.scope, func, self.hooks, &argv, name) {
            Ok(value) => value,
            Err(throw) => {
                self.thrown = Some(throw);
                return Err(Ended);
            }
        };
        if value.is_promise() {
            let feature = format!("{name} returned a promise; a synchronous answer is required");
            self.thrown = Some(callback_v8::not_impl(self.scope, &feature));
            return Err(Ended);
        }
        Ok(serde_v8::from_v8(self.scope, value).unwrap_or(Value::Null))
    }
}
