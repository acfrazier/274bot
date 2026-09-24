//! Typed V8 marshalling for v1 `PartnerTrade.js`: one native call per export
//! (`__rs2b0t_partner_trade(name, …args)`), deciding in `crate::partner_trade`.

use crate::partner_trade::{self as pt, MuleMode, OfferSlot, ReceiverOfferDecision};
use rustyscript::Runtime;

const PENDING: &str = "__pending__";

pub(super) fn install(runtime: &mut Runtime) -> Result<(), String> {
    let context = runtime.deno_runtime().main_context();
    let mut scope = runtime.deno_runtime().handle_scope();
    let global = context.open(&mut scope).global(&mut scope);
    let name = v8::String::new(&mut scope, "__rs2b0t_partner_trade")
        .ok_or_else(|| "partner trade name".to_string())?;
    let func = v8::Function::new(&mut scope, partner_trade_callback)
        .ok_or_else(|| "partner trade fn".to_string())?;
    global
        .set(&mut scope, name.into(), func.into())
        .ok_or_else(|| "partner trade set".to_string())?;
    Ok(())
}

fn partner_trade_callback(
    scope: &mut v8::HandleScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    match run(scope, &args) {
        Ok(value) => rv.set(value),
        Err(err) if err == PENDING => {}
        Err(err) => throw_error(scope, &err),
    }
}

fn run<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: &v8::FunctionCallbackArguments,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let export = args.get(0).to_rust_string_lossy(scope);
    let value: v8::Local<v8::Value> = match export.as_str() {
        "DEFAULT_TRADE_RANGE" => v8::Integer::new(scope, pt::DEFAULT_TRADE_RANGE).into(),
        "MULE_MODE_OPTIONS" => {
            let labels = pt::MULE_MODE_OPTIONS.map(str::to_string);
            string_array_value(scope, &labels)?
        }
        "parsePartnerList" => {
            let raw = js_string(scope, args.get(1))?;
            string_array_value(scope, &pt::parse_partner_list(&raw))?
        }
        "namesMatch" => {
            let a = js_string(scope, args.get(1))?;
            let b = js_string(scope, args.get(2))?;
            v8::Boolean::new(scope, pt::names_match(&a, &b)).into()
        }
        "isConfiguredPartner" => {
            let name = optional_string(scope, args.get(1))?;
            let partners = string_array(scope, args.get(2))?;
            v8::Boolean::new(scope, pt::is_configured_partner(name.as_deref(), &partners)).into()
        }
        "countOfferByName" => {
            let items = offer_slots(scope, args.get(1))?;
            let item_name = js_string(scope, args.get(2))?;
            v8::Number::new(scope, pt::count_offer_by_name(&items, &item_name)).into()
        }
        "decideReceiverOfferScreen" => receiver_decision(scope, args.get(1))?,
        "decideGiverOfferScreen" => {
            let slots = number(scope, args.get(1));
            v8_str(scope, pt::decide_giver_offer_screen(slots))?
        }
        "parseMuleMode" => {
            let raw = js_string(scope, args.get(1))?;
            v8_str(scope, pt::parse_mule_mode(&raw).as_str())?
        }
        "muleGathererHandoffActive"
        | "muleReceiverActive"
        | "muleCookerActive"
        | "muleSupplierActive"
        | "muleNonGathererActive" => {
            let mode = mule_mode(scope, args.get(1));
            let partners = array_len(args.get(2));
            let power_mode = args.get(3).boolean_value(scope);
            let active = match export.as_str() {
                "muleGathererHandoffActive" => {
                    pt::mule_gatherer_handoff_active(mode, partners, power_mode)
                }
                "muleReceiverActive" => pt::mule_receiver_active(mode, partners),
                "muleCookerActive" => pt::mule_cooker_active(mode, partners),
                "muleSupplierActive" => pt::mule_supplier_active(mode, partners, power_mode),
                _ => pt::mule_non_gatherer_active(mode, partners),
            };
            v8::Boolean::new(scope, active).into()
        }
        _ => return Err(format!("not impl: PartnerTrade.{export}")),
    };
    Ok(value)
}

/// `{ partnerHeader, partners, myOfferSlots, theirProductCount }` to the
/// frozen `{ action, reason? }` shape.
fn receiver_decision<'s>(
    scope: &mut v8::HandleScope<'s>,
    opts: v8::Local<v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let header = field(scope, opts, "partnerHeader")?;
    let header = optional_string(scope, header)?;
    let partners = field(scope, opts, "partners")?;
    let partners = string_array(scope, partners)?;
    let mine = field(scope, opts, "myOfferSlots")?;
    let mine = number(scope, mine);
    let theirs = field(scope, opts, "theirProductCount")?;
    let theirs = number(scope, theirs);
    let (action, reason) =
        match pt::decide_receiver_offer_screen(header.as_deref(), &partners, mine, theirs) {
            ReceiverOfferDecision::WaitHeader => ("wait-header", None),
            ReceiverOfferDecision::Decline(reason) => ("decline", Some(reason)),
            ReceiverOfferDecision::WaitOffer => ("wait-offer", None),
            ReceiverOfferDecision::Accept => ("accept", None),
        };
    let out = v8::Object::new(scope);
    let action = v8_str(scope, action)?;
    set_key(scope, out, "action", action);
    if let Some(reason) = reason {
        let reason = v8_str(scope, &reason)?;
        set_key(scope, out, "reason", reason);
    }
    Ok(out.into())
}

/// A non-string mode matches no role, as the frozen `===` checks.
fn mule_mode(scope: &mut v8::HandleScope, value: v8::Local<v8::Value>) -> MuleMode {
    if !value.is_string() {
        return MuleMode::Off;
    }
    MuleMode::from_mode(&value.to_rust_string_lossy(scope))
}

fn offer_slots(
    scope: &mut v8::HandleScope,
    value: v8::Local<v8::Value>,
) -> Result<Vec<OfferSlot>, String> {
    let Ok(array) = v8::Local::<v8::Array>::try_from(value) else {
        return Ok(Vec::new());
    };
    let mut out = Vec::with_capacity(array.length() as usize);
    for i in 0..array.length() {
        let row = array
            .get_index(scope, i)
            .ok_or_else(|| PENDING.to_string())?;
        let name = field(scope, row, "name")?;
        let name = optional_string(scope, name)?;
        let count = field(scope, row, "count")?;
        let count = number(scope, count);
        out.push(OfferSlot { name, count });
    }
    Ok(out)
}

fn array_len(value: v8::Local<v8::Value>) -> usize {
    v8::Local::<v8::Array>::try_from(value).map_or(0, |array| array.length() as usize)
}

fn number(scope: &mut v8::HandleScope, value: v8::Local<v8::Value>) -> f64 {
    value.number_value(scope).unwrap_or(f64::NAN)
}

fn js_string(scope: &mut v8::HandleScope, value: v8::Local<v8::Value>) -> Result<String, String> {
    let text = value.to_string(scope).ok_or_else(|| PENDING.to_string())?;
    Ok(text.to_rust_string_lossy(scope))
}

fn optional_string(
    scope: &mut v8::HandleScope,
    value: v8::Local<v8::Value>,
) -> Result<Option<String>, String> {
    if value.is_null_or_undefined() {
        return Ok(None);
    }
    js_string(scope, value).map(Some)
}

fn string_array(
    scope: &mut v8::HandleScope,
    value: v8::Local<v8::Value>,
) -> Result<Vec<String>, String> {
    let Ok(array) = v8::Local::<v8::Array>::try_from(value) else {
        return Ok(Vec::new());
    };
    let mut out = Vec::with_capacity(array.length() as usize);
    for i in 0..array.length() {
        let item = array
            .get_index(scope, i)
            .ok_or_else(|| PENDING.to_string())?;
        out.push(js_string(scope, item)?);
    }
    Ok(out)
}

fn string_array_value<'s>(
    scope: &mut v8::HandleScope<'s>,
    values: &[String],
) -> Result<v8::Local<'s, v8::Value>, String> {
    let array = v8::Array::new(scope, values.len() as i32);
    for (i, value) in values.iter().enumerate() {
        let value = v8_str(scope, value)?;
        array
            .set_index(scope, i as u32, value)
            .ok_or_else(|| "array".to_string())?;
    }
    Ok(array.into())
}

fn field<'s>(
    scope: &mut v8::HandleScope<'s>,
    value: v8::Local<v8::Value>,
    name: &str,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let Some(obj) = value.to_object(scope) else {
        return Ok(v8::undefined(scope).into());
    };
    let key = v8::String::new(scope, name).ok_or_else(|| "string".to_string())?;
    obj.get(scope, key.into())
        .ok_or_else(|| PENDING.to_string())
}

fn v8_str<'s>(
    scope: &mut v8::HandleScope<'s>,
    s: &str,
) -> Result<v8::Local<'s, v8::Value>, String> {
    v8::String::new(scope, s)
        .map(Into::into)
        .ok_or_else(|| "string".to_string())
}

fn set_key(
    scope: &mut v8::HandleScope,
    obj: v8::Local<v8::Object>,
    key: &str,
    value: v8::Local<v8::Value>,
) {
    if let Some(k) = v8::String::new(scope, key) {
        let _ = obj.set(scope, k.into(), value);
    }
}

fn throw_error(scope: &mut v8::HandleScope, message: &str) {
    if let Some(s) = v8::String::new(scope, message) {
        let exc = v8::Exception::error(scope, s);
        scope.throw_exception(exc);
    }
}
