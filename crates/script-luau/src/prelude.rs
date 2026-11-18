pub(crate) const PRELUDE: &str = r###"
local native_type = type
local native_error = error
local native_pcall = pcall
local native_yield = coroutine.yield
local native_tonumber = tonumber
local native_select = select
local native_tostring = tostring
local native_next = next
local native_rawget = rawget
local native_rawlen = rawlen
local native_floor = math.floor
local native_huge = math.huge

local native_string = string
local native_string_byte = native_string.byte
local native_string_format = native_string.format
local native_string_find = native_string.find
local native_string_match = native_string.match
local native_string_gmatch = native_string.gmatch
local native_string_gsub = native_string.gsub
local native_string_split = native_string.split
local native_table = table
local native_table_sort = native_table.sort
local native_table_find = native_table.find

local N_S = 262144
local N_P = 256
local W_P = 4194304
local W_N = 268435456
local N_R = 4096
local N_F = 1024
local N_A = 262144
local N_V = 8000
local N_C = 16384
local N_M = 65536

local function close_library(library, names)
    local allowed = {}
    for i = 1, #names do
        allowed[names[i]] = true
    end
    for key in native_next, library do
        if allowed[key] ~= true then
            library[key] = nil
        end
    end
end

local function number_guard(fn, label, positions, all_arguments, max_arguments)
    local bad_string = label .. ": string is not allowed for a numeric argument"
    local too_many = label .. ": argument count exceeds N_v (8000)"
    return function(...)
        local count = native_select("#", ...)
        if max_arguments ~= nil and count > max_arguments then
            native_error(too_many, 2)
        end
        if all_arguments then
            for i = 1, count do
                if native_type(native_select(i, ...)) == "string" then
                    native_error(bad_string, 2)
                end
            end
        else
            for i = 1, #positions do
                if native_type(native_select(positions[i], ...)) == "string" then
                    native_error(bad_string, 2)
                end
            end
        end
        return fn(...)
    end
end

local function guard_positions(library, name, positions)
    local fn = library[name]
    if native_type(fn) == "function" then
        library[name] = number_guard(fn, name, positions, false, nil)
    end
end

local function guard_all_numbers(library, name, max_arguments)
    local fn = library[name]
    if native_type(fn) == "function" then
        library[name] = number_guard(fn, name, nil, true, max_arguments)
    end
end

local function string_size(value)
    local kind = native_type(value)
    if kind == "string" then
        return #value
    end
    if kind == "number" then
        return #native_tostring(value)
    end
    return 0
end

local function search_refusal(name, subject, pattern, pattern_limit, work_limit)
    local subject_size = string_size(subject)
    local pattern_size = string_size(pattern)
    if subject_size > N_S then
        return name .. ": subject exceeds N_s (262144 bytes)"
    end
    if pattern_limit ~= nil and pattern_size > N_P then
        return name .. ": pattern exceeds N_p (256 bytes)"
    end
    if subject_size * pattern_size > work_limit then
        local cap = work_limit == W_N and "W_n (268435456)" or "W_p (4194304)"
        return name .. ": subject-by-pattern work exceeds " .. cap
    end
    return nil
end
local function integer_argument(value)
    if native_type(value) ~= "number" or value ~= value or value == native_huge or value == -native_huge then
        return nil
    end
    local converted
    if value < 0 then
        converted = -native_floor(-value)
    else
        converted = native_floor(value)
    end
    if converted < -2147483648 or converted > 2147483647 then
        return nil
    end
    return converted
end


local function format_string_argument_bytes(format, ...)
    local total = 0
    local argument = 0
    local i = 1
    local format_size = #format
    while i <= format_size do
        if native_string_byte(format, i) ~= 37 then
            i = i + 1
        else
            i = i + 1
            local current = native_string_byte(format, i)
            if current == 37 then
                i = i + 1
            elseif current == 42 then
                argument = argument + 1
                local value = native_select(argument, ...)
                if native_type(value) == "string" then
                    total = total + #value
                end
                i = i + 1
            elseif current ~= nil then
                while current == 45 or current == 43 or current == 32 or current == 35 or current == 48 do
                    i = i + 1
                    current = native_string_byte(format, i)
                end
                while current ~= nil and current >= 48 and current <= 57 do
                    i = i + 1
                    current = native_string_byte(format, i)
                end
                if current == 46 then
                    i = i + 1
                    current = native_string_byte(format, i)
                    while current ~= nil and current >= 48 and current <= 57 do
                        i = i + 1
                        current = native_string_byte(format, i)
                    end
                end
                if current ~= nil then
                    argument = argument + 1
                    if current == 113 or current == 115 then
                        local value = native_select(argument, ...)
                        if native_type(value) == "string" then
                            total = total + #value
                        end
                    end
                    i = i + 1
                end
            end
        end
        if total > N_A then
            return total
        end
    end
    return total
end

return function(host)
    if native_type(host) ~= "table" then
        native_error("Luau prelude host must be a table", 0)
    end

    local host_guard = host.guard
    local host_print = host.print
    local host_format = host.format
    local raw_bindings = host.bindings
    local waiting = host.waiting
    if native_type(host_guard) ~= "function" then
        native_error("Luau prelude host.guard must be a function", 0)
    end
    if native_type(host_print) ~= "function" then
        native_error("Luau prelude host.print must be a function", 0)
    end
    if native_type(raw_bindings) ~= "table" then
        native_error("Luau prelude host.bindings must be a table", 0)
    end
    if waiting ~= nil and native_type(waiting) ~= "table" then
        native_error("Luau prelude host.waiting must be a table", 0)
    end

    local function finish_binding(wait_for_resume, status, ...)
        if status == false then
            local message = (...)
            if native_type(message) ~= "string" then
                native_error("host binding failure did not include a string message", 3)
            end
            native_error(message, 3)
        end
        if status ~= true then
            native_error("host binding returned an invalid status", 3)
        end
        if wait_for_resume then
            return native_yield()
        end
        return ...
    end

    local function wrap_binding(fn, wait_for_resume)
        if native_type(fn) ~= "function" then
            native_error("Luau prelude binding leaf must be a function", 0)
        end
        return function(...)
            return finish_binding(wait_for_resume, fn(...))
        end
    end

    local function wrap_tree(source, wait_tree)
        if wait_tree ~= nil and native_type(wait_tree) ~= "table" then
            native_error("Luau prelude waiting tree does not match bindings", 0)
        end
        local wrapped = {}
        for key, value in native_next, source do
            local wait_value = nil
            if wait_tree ~= nil then
                wait_value = native_rawget(wait_tree, key)
            end
            local kind = native_type(value)
            if kind == "function" then
                if wait_value ~= nil and native_type(wait_value) ~= "boolean" then
                    native_error("Luau prelude waiting leaf must be boolean", 0)
                end
                wrapped[key] = wrap_binding(value, wait_value == true)
            elseif kind == "table" then
                wrapped[key] = wrap_tree(value, wait_value)
            else
                wrapped[key] = value
            end
        end
        return wrapped
    end

    local function guard_caught_error(value)
        local status, message = host_guard(value)
        if status == false then
            if native_type(message) ~= "string" then
                native_error("host guard failure did not include a string message", 4)
            end
            native_error(message, 4)
        end
        if status ~= true then
            native_error("host guard returned an invalid status", 4)
        end
    end

    local function protected_result(status, ...)
        if not status then
            guard_caught_error((...))
        end
        return status, ...
    end

    local function guarded_pcall(...)
        return protected_result(native_pcall(...))
    end

    local function xpcall_result(handler, status, ...)
        if status then
            return true, ...
        end
        local original_error = (...)
        guard_caught_error(original_error)
        local handler_status, handler_result = native_pcall(handler, original_error)
        if not handler_status then
            guard_caught_error(handler_result)
            return false, "error in error handling"
        end
        return false, handler_result
    end

    local function guarded_xpcall(fn, handler, ...)
        if native_type(handler) ~= "function" then
            native_error("bad argument #2 to 'xpcall' (function expected)", 2)
        end
        return xpcall_result(handler, native_pcall(fn, ...))
    end

    _G.pcall = guarded_pcall
    _G.xpcall = guarded_xpcall
    _G.print = wrap_binding(host_print, false)

    local string_library = string
    local table_library = table
    local math_library = math
    local bit_library = bit32
    local utf8_library = utf8
    local buffer_library = buffer

    close_library(math_library, {
        "abs", "acos", "asin", "atan2", "atan", "ceil", "cosh", "cos", "deg", "exp", "floor", "fmod",
        "frexp", "ldexp", "log10", "log", "max", "min", "modf", "pow", "rad", "random", "randomseed",
        "sinh", "sin", "sqrt", "tanh", "tan", "noise", "clamp", "sign", "round", "map", "lerp",
        "isnan", "isinf", "isfinite", "pi", "huge", "nan", "e", "phi", "sqrt2", "tau",
    })
    close_library(bit_library, {
        "arshift", "band", "bnot", "bor", "bxor", "btest", "extract", "lrotate", "lshift", "replace",
        "rrotate", "rshift", "countlz", "countrz", "byteswap",
    })
    close_library(string_library, {
        "byte", "char", "find", "format", "gmatch", "gsub", "len", "lower", "match", "rep", "reverse",
        "sub", "upper", "split",
    })
    close_library(table_library, {
        "concat", "getn", "maxn", "insert", "remove", "sort", "pack", "unpack", "move", "create", "find",
        "clear", "freeze", "isfrozen", "clone",
    })
    close_library(utf8_library, {"offset", "codepoint", "char", "len", "codes", "charpattern"})
    close_library(buffer_library, {
        "create", "fromstring", "tostring", "readi8", "readu8", "readi16", "readu16", "readi32", "readu32",
        "readf32", "readf64", "writei8", "writeu8", "writei16", "writeu16", "writei32", "writeu32", "writef32",
        "writef64", "readstring", "writestring", "len", "copy", "fill", "readbits", "writebits", "readinteger",
        "writeinteger",
    })

    guard_positions(math_library, "abs", {1})
    guard_positions(math_library, "acos", {1})
    guard_positions(math_library, "asin", {1})
    guard_positions(math_library, "atan2", {1, 2})
    guard_positions(math_library, "atan", {1})
    guard_positions(math_library, "ceil", {1})
    guard_positions(math_library, "cosh", {1})
    guard_positions(math_library, "cos", {1})
    guard_positions(math_library, "deg", {1})
    guard_positions(math_library, "exp", {1})
    guard_positions(math_library, "floor", {1})
    guard_positions(math_library, "fmod", {1, 2})
    guard_positions(math_library, "frexp", {1})
    guard_positions(math_library, "ldexp", {1, 2})
    guard_positions(math_library, "log10", {1})
    guard_positions(math_library, "log", {1, 2})
    guard_all_numbers(math_library, "max", N_V)
    guard_all_numbers(math_library, "min", N_V)
    guard_positions(math_library, "modf", {1})
    guard_positions(math_library, "pow", {1, 2})
    guard_positions(math_library, "rad", {1})
    guard_positions(math_library, "random", {1, 2})
    guard_positions(math_library, "randomseed", {1})
    guard_positions(math_library, "sinh", {1})
    guard_positions(math_library, "sin", {1})
    guard_positions(math_library, "sqrt", {1})
    guard_positions(math_library, "tanh", {1})
    guard_positions(math_library, "tan", {1})
    guard_positions(math_library, "noise", {1, 2, 3})
    guard_positions(math_library, "clamp", {1, 2, 3})
    guard_positions(math_library, "sign", {1})
    guard_positions(math_library, "round", {1})
    guard_positions(math_library, "map", {1, 2, 3, 4, 5})
    guard_positions(math_library, "lerp", {1, 2, 3})
    guard_positions(math_library, "isnan", {1})
    guard_positions(math_library, "isinf", {1})
    guard_positions(math_library, "isfinite", {1})

    guard_positions(bit_library, "arshift", {1, 2})
    guard_all_numbers(bit_library, "band", N_V)
    guard_positions(bit_library, "bnot", {1})
    guard_all_numbers(bit_library, "bor", N_V)
    guard_all_numbers(bit_library, "bxor", N_V)
    guard_all_numbers(bit_library, "btest", N_V)
    guard_positions(bit_library, "extract", {1, 2, 3})
    guard_positions(bit_library, "lrotate", {1, 2})
    guard_positions(bit_library, "lshift", {1, 2})
    guard_positions(bit_library, "replace", {1, 2, 3, 4})
    guard_positions(bit_library, "rrotate", {1, 2})
    guard_positions(bit_library, "rshift", {1, 2})
    guard_positions(bit_library, "countlz", {1})
    guard_positions(bit_library, "countrz", {1})
    guard_positions(bit_library, "byteswap", {1})

    guard_positions(string_library, "byte", {2, 3})
    guard_all_numbers(string_library, "char", N_V)
    guard_positions(string_library, "gsub", {4})
    guard_positions(string_library, "match", {3})
    guard_positions(string_library, "rep", {2})
    guard_positions(string_library, "sub", {2, 3})

    local native_insert = table_library.insert
    table_library.insert = function(...)
        if native_select("#", ...) >= 3 and native_type(native_select(2, ...)) == "string" then
            native_error("table.insert: string is not allowed for a numeric argument", 2)
        end
        return native_insert(...)
    end
    guard_positions(table_library, "remove", {2})
    guard_positions(table_library, "unpack", {2, 3})
    guard_positions(table_library, "create", {1})

    guard_positions(utf8_library, "offset", {2, 3})
    guard_positions(utf8_library, "codepoint", {2, 3})
    guard_all_numbers(utf8_library, "char", N_V)
    guard_positions(utf8_library, "len", {2, 3})
    guard_positions(utf8_library, "codes", {2, 3})

    guard_positions(buffer_library, "create", {1})
    guard_positions(buffer_library, "readi8", {2})
    guard_positions(buffer_library, "readu8", {2})
    guard_positions(buffer_library, "readi16", {2})
    guard_positions(buffer_library, "readu16", {2})
    guard_positions(buffer_library, "readi32", {2})
    guard_positions(buffer_library, "readu32", {2})
    guard_positions(buffer_library, "readf32", {2})
    guard_positions(buffer_library, "readf64", {2})
    guard_positions(buffer_library, "writei8", {2, 3})
    guard_positions(buffer_library, "writeu8", {2, 3})
    guard_positions(buffer_library, "writei16", {2, 3})
    guard_positions(buffer_library, "writeu16", {2, 3})
    guard_positions(buffer_library, "writei32", {2, 3})
    guard_positions(buffer_library, "writeu32", {2, 3})
    guard_positions(buffer_library, "writef32", {2, 3})
    guard_positions(buffer_library, "writef64", {2, 3})
    guard_positions(buffer_library, "readstring", {2, 3})
    guard_positions(buffer_library, "writestring", {2, 4})
    guard_positions(buffer_library, "copy", {2, 4, 5})
    guard_positions(buffer_library, "fill", {2, 3, 4})
    guard_positions(buffer_library, "readbits", {2, 3})
    guard_positions(buffer_library, "writebits", {2, 3, 4})
    guard_positions(buffer_library, "readinteger", {2, 3})
    guard_positions(buffer_library, "writeinteger", {2, 3, 4})

    local native_string_gsub_guarded = number_guard(native_string_gsub, "string.gsub", {4}, false, nil)
    local native_string_find_guarded = number_guard(native_string_find, "string.find", {3}, false, nil)
    local native_string_match_guarded = number_guard(native_string_match, "string.match", {3}, false, nil)
    local native_string_gmatch = string_library.gmatch
    local native_string_split = string_library.split

    string_library.find = function(subject, pattern, init, plain)
        local work_limit = plain and W_N or W_P
        local pattern_limit = N_P
        if plain then
            pattern_limit = nil
        end
        local refusal = search_refusal("string.find", subject, pattern, pattern_limit, work_limit)
        if refusal ~= nil then
            native_error(refusal, 2)
        end
        return native_string_find_guarded(subject, pattern, init, plain)
    end
    string_library.match = function(subject, pattern, init)
        local refusal = search_refusal("string.match", subject, pattern, N_P, W_P)
        if refusal ~= nil then
            native_error(refusal, 2)
        end
        return native_string_match_guarded(subject, pattern, init)
    end
    string_library.gmatch = function(subject, pattern)
        local refusal = search_refusal("string.gmatch", subject, pattern, N_P, W_P)
        if refusal ~= nil then
            native_error(refusal, 2)
        end
        return native_string_gmatch(subject, pattern)
    end
    string_library.gsub = function(subject, pattern, replacement, count)
        local refusal = search_refusal("string.gsub", subject, pattern, N_P, W_P)
        if refusal ~= nil then
            native_error(refusal, 2)
        end
        if native_type(replacement) == "string" and #replacement > N_R then
            native_error("string.gsub: replacement exceeds N_r (4096 bytes)", 2)
        end
        return native_string_gsub_guarded(subject, pattern, replacement, count)
    end
    string_library.split = function(subject, separator)
        local refusal = search_refusal("string.split", subject, separator, nil, W_N)
        if refusal ~= nil then
            native_error(refusal, 2)
        end
        return native_string_split(subject, separator)
    end
    string_library.format = function(format, ...)
        if native_type(format) == "string" then
            if #format > N_F then
                native_error("string.format: format exceeds N_f (1024 bytes)", 2)
            end
            if format_string_argument_bytes(format, ...) > N_A then
                native_error("string.format: string arguments exceed N_a (262144 bytes)", 2)
            end
        end
        if native_type(host_format) ~= "function" then
            native_error("string.format: host validator is unavailable", 2)
        end
        local status, message = host_format(format, ...)
        if status == false then
            if native_type(message) ~= "string" then
                native_error("string.format: host validator failed without a string message", 2)
            end
            native_error(message, 2)
        end
        if status ~= true then
            native_error("string.format: host validator returned an invalid status", 2)
        end
        return native_string_format(format, ...)
    end

    local native_table_concat = number_guard(table_library.concat, "table.concat", {3, 4}, false, nil)
    table_library.concat = function(value, separator, first, last)
        if native_type(value) == "table" then
            local start_value = first
            if start_value == nil then
                start_value = 1
            end
            local end_value = last
            if end_value == nil then
                end_value = native_rawlen(value)
            end
            if native_type(start_value) == "number" and native_type(end_value) == "number" then
                local start_index = integer_argument(start_value)
                local end_index = integer_argument(end_value)
                if start_index == nil or end_index == nil then
                    native_error("table.concat: index is outside the integer range", 2)
                end
                if end_index - start_index + 1 > N_C then
                    native_error("table.concat: field count exceeds N_c (16384)", 2)
                end
            end
        end
        return native_table_concat(value, separator, first, last)
    end
    local native_table_move = number_guard(table_library.move, "table.move", {2, 3, 4, 5}, false, nil)
    table_library.move = function(source, first, last, target, destination)
        if native_type(first) == "number" and native_type(last) == "number" then
            local first_index = integer_argument(first)
            local last_index = integer_argument(last)
            if first_index == nil or last_index == nil then
                native_error("table.move: index is outside the integer range", 2)
            end
            if last_index - first_index + 1 > N_M then
                native_error("table.move: element count exceeds N_m (65536)", 2)
            end
        end
        return native_table_move(source, first, last, target, destination)
    end


    local function guarded_find(value, needle, init)
        local kind = native_type(needle)
        if kind == "number" or kind == "string" or kind == "boolean" then
            return native_table_find(value, needle, init)
        end
        if native_type(value) ~= "table" then
            return native_table_find(value, needle, init)
        end
        local start_index = 1
        if init ~= nil then
            if native_type(init) ~= "number" then
                return native_table_find(value, needle, init)
            end
            start_index = integer_argument(init)
            if start_index == nil or start_index < 1 then
                native_error("bad argument #3 to 'find' (index out of range)", 3)
            end
        end
        local index = start_index
        while true do
            local element = native_rawget(value, index)
            if element == nil then
                return nil
            end
            if needle == element then
                return index
            end
            if index == 2147483647 then
                return nil
            end
            index = index + 1
        end
    end
    table_library.find = number_guard(guarded_find, "table.find", {3}, false, nil)

    table_library.sort = function(value, comparator)
        if comparator == nil then
            return native_table_sort(value, function(left, right)
                return left < right
            end)
        end
        if native_type(comparator) ~= "function" then
            return native_table_sort(value, comparator)
        end
        return native_table_sort(value, function(left, right)
            return comparator(left, right)
        end)
    end

    local native_select_guarded = function(index, ...)
        if native_type(index) == "string" and index ~= "#" then
            native_error("select: string index must be '#'", 2)
        end
        return native_select(index, ...)
    end
    _G.select = native_select_guarded
    _G.tonumber = function(value, base)
        if base ~= nil and native_type(base) == "string" then
            native_error("tonumber: base must be numeric", 2)
        end
        return native_tonumber(value, base)
    end
    _G.error = function(message, level)
        if level ~= nil and native_type(level) == "string" then
            native_error("error: level must be numeric", 2)
        end
        return native_error(message, level)
    end

    return wrap_tree(raw_bindings, waiting)
end
"###;
