include!("../src/prelude.rs");

use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use mlua::{Function, Lua, MultiValue, Table, Value};

fn host(lua: &Lua) -> mlua::Result<(Table, Arc<AtomicUsize>, Arc<AtomicUsize>)> {
    let host = lua.create_table()?;
    let guard_calls = Arc::new(AtomicUsize::new(0));
    let print_calls = Arc::new(AtomicUsize::new(0));

    let guard_counter = Arc::clone(&guard_calls);
    host.set(
        "guard",
        lua.create_function(move |_, _: Value| {
            guard_counter.fetch_add(1, Ordering::Relaxed);
            Ok((true,))
        })?,
    )?;

    let print_counter = Arc::clone(&print_calls);
    host.set(
        "print",
        lua.create_function(move |_, _: MultiValue| {
            print_counter.fetch_add(1, Ordering::Relaxed);
            Ok((true,))
        })?,
    )?;
    host.set(
        "format",
        lua.create_function(|_, _: MultiValue| Ok((true,)))?,
    )?;
    host.set("bindings", lua.create_table()?)?;

    Ok((host, guard_calls, print_calls))
}

fn run_prelude(lua: &Lua, host: Table) -> mlua::Result<Table> {
    let setup: Function = lua.load(PRELUDE).eval()?;
    setup.call(host)
}

#[test]
fn prelude_wraps_status_callbacks_waits_and_protected_calls() -> mlua::Result<()> {
    let lua = Lua::new();
    let (host, guard_calls, print_calls) = host(&lua)?;
    let bindings: Table = host.get("bindings")?;
    let settings = lua.create_table()?;
    settings.set(
        "number",
        lua.create_function(|_, _: MultiValue| Ok((true, 42)))?,
    )?;
    settings.set(
        "failure",
        lua.create_function(|_, _: MultiValue| Ok((false, "binding failure")))?,
    )?;
    settings.set("wait", lua.create_function(|_, _: MultiValue| Ok((true,)))?)?;
    bindings.set("settings", settings)?;

    let waiting = lua.create_table()?;
    let settings_waiting = lua.create_table()?;
    settings_waiting.set("wait", true)?;
    waiting.set("settings", settings_waiting)?;
    host.set("waiting", waiting)?;

    let api = run_prelude(&lua, host)?;
    lua.globals().set("api", api)?;
    lua.load(
        r#"
        local ok, value = pcall(api.settings.number)
        assert(ok and value == 42)

        local failed, message = pcall(api.settings.failure)
        assert(not failed and string.find(message, "binding failure", 1, true))

        local xok, xmessage = xpcall(function()
            error("original failure")
        end, function()
            error("handler failure")
        end)
        assert(not xok and xmessage == "error in error handling")

        local thread = coroutine.create(function()
            return api.settings.wait()
        end)
        local started = coroutine.resume(thread)
        assert(started)
        local resumed, result = coroutine.resume(thread, "delivered")
        assert(resumed and result == "delivered")

        print("logged")
        assert(string.format("%%:%*:%q:%02d", "raw", "quoted", 7) == "%:raw:\"quoted\":07")
        -- Luau treats `%*` as one conversion, so the following `s` is literal.
        assert(string.format("%*s", "raw", string.rep("x", 262145)) == "raws")
        local precision_star_ok = pcall(string.format, "%.*s", 1, "raw")
        assert(not precision_star_ok)
        "#,
    )
    .exec()?;

    assert!(guard_calls.load(Ordering::Relaxed) >= 3);
    assert_eq!(print_calls.load(Ordering::Relaxed), 1);
    Ok(())
}

#[test]
fn prelude_rejects_numeric_strings_and_closes_unsafe_library_fields() -> mlua::Result<()> {
    let lua = Lua::new();
    let (host, _, _) = host(&lua)?;
    lua.globals().set("api", run_prelude(&lua, host)?)?;
    lua.load(
        r#"
        local function rejected(fn)
            local ok, message = pcall(fn)
            assert(not ok and type(message) == "string")
        end

        rejected(function() return math.floor("3") end)
        rejected(function() return math.min(1, "2") end)
        rejected(function() return bit32.band("1", 1) end)
        rejected(function() return string.char("65") end)
        rejected(function() return string.sub("abc", "1", 2) end)
        rejected(function() return table.concat({}, ",", "1", 2) end)
        rejected(function() return table.move({}, "1", 1, 1, {}) end)
        rejected(function() return table.move({}, 1, 1, 1, "1") end)
        rejected(function() return table.find({}, "x", "1") end)
        rejected(function() return utf8.len("x", "1") end)
        rejected(function() return buffer.create("1") end)
        rejected(function() return tonumber("10", "2") end)
        assert(tonumber("10", 2) == 2)
        assert(tonumber("ff", 16) == 255)

        assert(table.foreach == nil and table.foreachi == nil)
        assert(string.pack == nil and string.unpack == nil and string.packsize == nil)

        local equality = { __eq = function() return true end }
        local item = setmetatable({}, equality)
        local needle = setmetatable({}, equality)
        local values = setmetatable({ item }, {
            __index = function() error("table.find used __index") end,
        })
        assert(table.find(values, needle) == 1)
        assert(table.find(values, needle, 1.5) == 1)
        local sparse = setmetatable({ item, [3] = item }, {
            __index = function() error("table.find used __index") end,
        })
        assert(table.find(sparse, needle, 2.5) == nil)

        local comparisons = 0
        local sorted = { 4, 1, 3, 2 }
        table.sort(sorted, function(left, right)
            comparisons = comparisons + 1
            return left < right
        end)
        assert(sorted[1] == 1 and sorted[4] == 4 and comparisons > 0)

        local default_sorted = { 3, 1, 2 }
        table.sort(default_sorted)
        assert(default_sorted[1] == 1 and default_sorted[3] == 3)
        "#,
    )
    .exec()?;
    Ok(())
}

#[test]
fn prelude_enforces_round_seven_native_call_caps() -> mlua::Result<()> {
    let lua = Lua::new();
    let (host, _, _) = host(&lua)?;
    lua.globals().set("api", run_prelude(&lua, host)?)?;
    lua.load(
        r#"
        local function rejected(cap, fn)
            local ok, message = pcall(fn)
            assert(not ok and string.find(message, cap, 1, true), message)
        end

        local subject = string.rep("a", 262144)
        local long_subject = subject .. "a"
        local long_pattern = string.rep("x", 257)
        local large_plain_needle = string.rep("x", 1025)

        rejected("N_s", function() string.find(long_subject, "a") end)
        rejected("N_p", function() string.match("", long_pattern) end)
        rejected("W_p", function() string.find(subject, string.rep("x", 17)) end)
        rejected("W_n", function() string.find(subject, large_plain_needle, 1, true) end)
        rejected("W_n", function() string.split(subject, large_plain_needle) end)
        rejected("N_r", function() string.gsub("", "x", string.rep("x", 4097)) end)
        rejected("N_f", function() string.format(string.rep("x", 1025)) end)
        rejected("N_a", function() string.format("%s", string.rep("x", 262145)) end)
        rejected("N_a", function() string.format("%*", string.rep("x", 262145)) end)
        rejected("N_c", function() table.concat({}, ",", 1.5, 16386.5) end)
        rejected("N_m", function() table.move({}, 1.5, 65538.5, 1, {}) end)

        local arguments = table.create(8001, 1)
        local guarded = {
            math.min, math.max, bit32.band, bit32.bor, bit32.bxor, bit32.btest,
            string.char, utf8.char,
        }
        for _, fn in ipairs(guarded) do
            rejected("N_v", function()
                return fn(table.unpack(arguments, 1, 8001))
            end)
        end
        "#,
    )
    .exec()?;
    Ok(())
}

#[test]
fn prelude_fails_closed_on_non_text_format_errors() -> mlua::Result<()> {
    let lua = Lua::new();
    let (host, _, _) = host(&lua)?;
    host.set(
        "format",
        lua.create_function(|lua, format: String| {
            if format == "text-error" {
                Ok((
                    false,
                    Value::String(lua.create_string("validator rejection")?),
                ))
            } else {
                Ok((false, Value::Integer(42)))
            }
        })?,
    )?;
    lua.globals().set("api", run_prelude(&lua, host)?)?;
    lua.load(
        r#"
        local text_ok, text_message = pcall(string.format, "text-error")
        assert(not text_ok and text_message == "validator rejection")

        local value_ok, value_message = pcall(string.format, "non-text-error")
        assert(not value_ok and type(value_message) == "string")
        assert(string.find(value_message, "without a string message", 1, true))
        "#,
    )
    .exec()?;
    Ok(())
}
