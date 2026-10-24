use deno_core::{op2, v8};
use rand::RngCore;

const MAX_RANDOM_VALUES_BYTES: usize = 65_536;

deno_core::extension!(
    script_crypto,
    ops = [op_crypto_random_uuid, op_crypto_get_random_values],
    esm_entry_point = "ext:script_crypto/crypto_stub.js",
    esm = [dir "src/runtime", "crypto_stub.js"],
);

#[op2]
fn op_crypto_random_uuid<'s>(scope: &mut v8::HandleScope<'s>) -> v8::Local<'s, v8::Value> {
    let mut bytes = [0; 16];
    let mut rng = rand::rngs::OsRng;
    if rng.try_fill_bytes(&mut bytes).is_err() {
        return throw_error(scope, "Secure random number generation failed");
    }

    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;

    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut uuid = [0; 36];
    let mut position = 0;
    for (index, byte) in bytes.into_iter().enumerate() {
        if matches!(index, 4 | 6 | 8 | 10) {
            uuid[position] = b'-';
            position += 1;
        }
        uuid[position] = HEX[(byte >> 4) as usize];
        uuid[position + 1] = HEX[(byte & 0x0f) as usize];
        position += 2;
    }

    let uuid = std::str::from_utf8(&uuid).expect("UUID bytes are ASCII");
    v8::String::new(scope, uuid)
        .expect("V8 can allocate the UUID string")
        .into()
}

#[op2]
fn op_crypto_get_random_values<'s>(
    scope: &mut v8::HandleScope<'s>,
    value: v8::Local<'s, v8::Value>,
    #[anybuffer] bytes: &mut [u8],
) -> v8::Local<'s, v8::Value> {
    if !value.is_array_buffer_view() {
        return throw_type_error(scope, "The argument must be an ArrayBufferView");
    }
    if !is_integer_typed_array(value) {
        return throw_dom_exception(
            scope,
            "TypeMismatchError",
            "The provided ArrayBufferView is not an integer array type",
        );
    }
    if bytes.len() > MAX_RANDOM_VALUES_BYTES {
        let message = format!(
            "The ArrayBufferView's byte length ({}) exceeds the number of bytes of entropy available via this API ({MAX_RANDOM_VALUES_BYTES})",
            bytes.len(),
        );
        return throw_dom_exception(scope, "QuotaExceededError", &message);
    }

    let mut rng = rand::rngs::OsRng;
    if rng.try_fill_bytes(bytes).is_err() {
        return throw_error(scope, "Secure random number generation failed");
    }
    value
}

fn is_integer_typed_array(value: v8::Local<v8::Value>) -> bool {
    value.is_int8_array()
        || value.is_uint8_array()
        || value.is_uint8_clamped_array()
        || value.is_int16_array()
        || value.is_uint16_array()
        || value.is_int32_array()
        || value.is_uint32_array()
        || value.is_big_int64_array()
        || value.is_big_uint64_array()
}

fn throw_dom_exception<'s>(
    scope: &mut v8::HandleScope<'s>,
    name: &str,
    message: &str,
) -> v8::Local<'s, v8::Value> {
    let global = scope.get_current_context().global(scope);
    let key = v8::String::new(scope, "DOMException").unwrap();
    let constructor = global.get(scope, key.into()).and_then(|value| {
        if let Ok(constructor) = v8::Local::<v8::Function>::try_from(value) {
            return Some(constructor);
        }
        let namespace = v8::Local::<v8::Object>::try_from(value).ok()?;
        let key = v8::String::new(scope, "DOMException")?;
        let value = namespace.get(scope, key.into())?;
        v8::Local::<v8::Function>::try_from(value).ok()
    });
    if let Some(constructor) = constructor {
        let message = v8::String::new(scope, message).unwrap();
        let name = v8::String::new(scope, name).unwrap();
        let arguments: [v8::Local<v8::Value>; 2] = [message.into(), name.into()];
        if let Some(exception) = constructor.new_instance(scope, &arguments) {
            scope.throw_exception(exception.into());
            return v8::undefined(scope).into();
        }
    }
    throw_error(scope, "DOMException is unavailable")
}

fn throw_type_error<'s>(
    scope: &mut v8::HandleScope<'s>,
    message: &str,
) -> v8::Local<'s, v8::Value> {
    let message = v8::String::new(scope, message).unwrap();
    let exception = v8::Exception::type_error(scope, message);
    scope.throw_exception(exception);
    v8::undefined(scope).into()
}

fn throw_error<'s>(scope: &mut v8::HandleScope<'s>, message: &str) -> v8::Local<'s, v8::Value> {
    let message = v8::String::new(scope, message).unwrap();
    let exception = v8::Exception::error(scope, message);
    scope.throw_exception(exception);
    v8::undefined(scope).into()
}
