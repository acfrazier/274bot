use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use mlua::{Error, Function, Lua, Table, Value, VmState};

const MLUA_VERSION: &str = "0.12.2";
const LUAU_SOURCE_VERSION: &str = "0.22.0+luau740";
const WATCHDOG_MARKER: &str = "L0_LUAU_PROBE_WATCHDOG";
const YIELD_BOUNDARY_ERROR: &str = "attempt to yield across metamethod/C-call boundary";

pub fn run_cli() -> ExitCode {
    println!("mlua={MLUA_VERSION} luau0-src={LUAU_SOURCE_VERSION}");
    let mut failed = false;
    match probe_callback_yield() {
        Ok(result) => println!("yield_across_rust_callback={result}"),
        Err(error) => {
            println!("yield_across_rust_callback=ERROR {error}");
            failed = true;
        }
    }
    report("memory_limit", probe_memory_limit(), &mut failed);
    report("watchdog_interrupt", probe_interrupt(), &mut failed);
    report("scoped_borrow", probe_scoped_borrow(), &mut failed);
    report(
        "nested_pcall_interrupt",
        probe_nested_pcall_interrupt(),
        &mut failed,
    );

    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn report(name: &str, result: Result<String, String>, failed: &mut bool) {
    match result {
        Ok(details) => println!("{name}=PASS {details}"),
        Err(error) => {
            println!("{name}=FAIL {error}");
            *failed = true;
        }
    }
}
fn probe_callback_yield() -> Result<String, String> {
    let lua = Lua::new();
    let callback = lua
        .create_function(|lua, ()| {
            let coroutine: Table = lua.globals().get("coroutine")?;
            let yield_function: Function = coroutine.get("yield")?;
            let resumed_value: String = yield_function.call("yielded-from-rust-callback")?;
            Ok(resumed_value)
        })
        .map_err(|error| format!("create Rust callback: {error}"))?;
    lua.globals()
        .set("rust_callback", callback)
        .map_err(|error| format!("register Rust callback: {error}"))?;
    let entry: Function = lua
        .load("return function() return rust_callback() end")
        .eval()
        .map_err(|error| format!("compile coroutine: {error}"))?;
    let thread = lua
        .create_thread(entry)
        .map_err(|error| format!("create coroutine: {error}"))?;
    let yielded: String = match thread.resume(()) {
        Ok(yielded) => yielded,
        Err(error) if error.to_string().contains(YIELD_BOUNDARY_ERROR) => {
            return Ok(format!("NO error={error:?}"));
        }
        Err(error) => return Err(format!("unexpected coroutine resume error: {error}")),
    };
    let resumed: String = thread
        .resume("resumed-through-rust-callback")
        .map_err(|error| format!("coroutine did not resume: {error}"))?;
    if yielded != "yielded-from-rust-callback" || resumed != "resumed-through-rust-callback" {
        return Err(format!(
            "unexpected yielded={yielded:?} resumed={resumed:?}"
        ));
    }
    Ok(format!("YES yielded={yielded:?} resumed={resumed:?}"))
}

fn probe_memory_limit() -> Result<String, String> {
    let lua = Lua::new();
    let allocate = lua
        .load("local held = {}; for i = 1, 10000 do held[i] = {1, 2, 3, 4} end")
        .into_function()
        .map_err(|error| format!("compile allocator: {error}"))?;
    let memory_before_limit = lua.used_memory();
    let limit = memory_before_limit
        .checked_add(64 * 1024)
        .ok_or_else(|| "memory limit overflow".to_owned())?;
    let previous_limit = lua
        .set_memory_limit(limit)
        .map_err(|error| format!("set memory limit: {error}"))?;
    let result = allocate.call::<()>(());
    let used_after_error = lua.used_memory();
    match result {
        Err(Error::MemoryError(error)) => Ok(format!(
            "limit_bytes={limit} previous_limit_bytes={previous_limit} used_after_error_bytes={used_after_error} error={error:?}"
        )),
        Err(error) => Err(format!(
            "expected MemoryError at {limit} bytes, received {error}"
        )),
        Ok(()) => Err(format!("bounded allocator did not hit the {limit}-byte limit")),
    }
}

fn probe_interrupt() -> Result<String, String> {
    let lua = Lua::new();
    let calls = Arc::new(AtomicUsize::new(0));
    let interrupt_calls = Arc::clone(&calls);
    lua.set_interrupt(move |_| {
        interrupt_calls.fetch_add(1, Ordering::Relaxed);
        Err(Error::RuntimeError(WATCHDOG_MARKER.to_owned()))
    });
    let result = lua
        .load("local total = 0; for i = 1, 1000000 do total += i end; return total")
        .exec();
    lua.remove_interrupt();
    let count = calls.load(Ordering::Relaxed);
    match result {
        Err(error) if count > 0 && error.to_string().contains(WATCHDOG_MARKER) => {
            Ok(format!("callbacks={count} error={error:?}"))
        }
        Err(error) => Err(format!(
            "interrupt ran {count} times but did not return its watchdog error: {error}"
        )),
        Ok(()) => Err("bounded script completed without a watchdog error".to_owned()),
    }
}
fn probe_scoped_borrow() -> Result<String, String> {
    let lua = Lua::new();
    let borrowed = String::from("borrowed only during this scoped call");
    let expected_bytes = borrowed.len();
    let (escaped, observed_bytes) = lua
        .scope(|scope| {
            let callback = scope.create_function(|_, ()| Ok(borrowed.len()))?;
            lua.globals().set("borrowed_len", callback.clone())?;
            let observed: usize = lua.load("return borrowed_len()").eval()?;
            Ok((callback, observed))
        })
        .map_err(|error| format!("call through scoped borrow: {error}"))?;
    let escaped_result = escaped.call::<usize>(());
    lua.globals()
        .set("borrowed_len", Value::Nil)
        .map_err(|error| format!("remove scoped callback: {error}"))?;
    match escaped_result {
        Err(error) if callback_destructed(&error) && observed_bytes == expected_bytes => Ok(
            format!("borrowed_bytes={observed_bytes} after_scope=CallbackDestructed"),
        ),
        Err(error) => Err(format!("unexpected post-scope callback error: {error}")),
        Ok(value) => Err(format!(
            "borrowed callback escaped scope with value {value}; expected {expected_bytes} bytes"
        )),
    }
}
fn callback_destructed(error: &Error) -> bool {
    match error {
        Error::CallbackDestructed => true,
        Error::CallbackError { cause, .. } => callback_destructed(cause),
        Error::WithContext { cause, .. } => callback_destructed(cause),
        _ => false,
    }
}

fn probe_nested_pcall_interrupt() -> Result<String, String> {
    let lua = Lua::new();
    let calls = Arc::new(AtomicUsize::new(0));
    let armed = Arc::new(AtomicBool::new(false));
    let interrupt_calls = Arc::clone(&calls);
    let interrupt_armed = Arc::clone(&armed);
    lua.set_interrupt(move |_| {
        interrupt_calls.fetch_add(1, Ordering::Relaxed);
        if interrupt_armed.load(Ordering::Acquire) {
            Err(Error::RuntimeError(WATCHDOG_MARKER.to_owned()))
        } else {
            Ok(VmState::Continue)
        }
    });
    let arm_watchdog = Arc::clone(&armed);
    let callback = lua
        .create_function(move |_, ()| {
            arm_watchdog.store(true, Ordering::Release);
            Ok(())
        })
        .map_err(|error| format!("create watchdog arm callback: {error}"))?;
    lua.globals()
        .set("arm_watchdog", callback)
        .map_err(|error| format!("register watchdog arm callback: {error}"))?;
    let result = lua
        .load(
            r#"
            local outer_ok = pcall(function()
                local middle_ok = pcall(function()
                    local inner_ok = pcall(function()
                        arm_watchdog()
                        local total = 0
                        for i = 1, 1000000 do
                            total += i
                        end
                    end)
                end)
            end)
            return outer_ok
            "#,
        )
        .eval::<bool>();
    lua.remove_interrupt();
    let callback_count = calls.load(Ordering::Acquire);
    let was_armed = armed.load(Ordering::Acquire);
    match result {
        Err(error)
            if was_armed
                && callback_count > 0
                && error.to_string().contains(WATCHDOG_MARKER) =>
        {
            Ok(format!(
                "callbacks={callback_count} armed={was_armed} rust_error={error:?}"
            ))
        }
        Err(error) => Err(format!(
            "interrupt ran {callback_count} times with armed={was_armed} but Rust received no watchdog marker: {error}"
        )),
        Ok(outer_ok) => Err(format!(
            "nested pcall returned {outer_ok}; callbacks={callback_count} armed={was_armed}"
        )),
    }
}
