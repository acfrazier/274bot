//! Native ownership for the v1 GameMessages API.
//!
//! The current posted page remains the source for `mark`, `since`, and
//! `sawSince`. The bounded native ring owns only V8 references to its message
//! DTOs and the fixed metadata needed to order them.

use super::callback_v8::{self as cb, JsResult, Throw};
use rustyscript::deno_core::v8;
use rustyscript::Runtime;
use std::cell::RefCell;
use std::rc::Rc;

const CAPACITY: usize = 64;
const NAME: &str = "__rs2b0t_game_messages";
const NEGATE_SOURCE: &str = "(value) => -value";

const OP_RECORD: i32 = 0;
const OP_MARK: i32 = 1;
const OP_SINCE: i32 = 2;
const OP_SAW_SINCE: i32 = 3;
const OP_FIRST_SINCE: i32 = 4;
const OP_RECENT: i32 = 5;
const OP_RESET: i32 = 6;

const ARRAY_INDEXES: [&str; CAPACITY] = [
    "0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12", "13", "14", "15", "16",
    "17", "18", "19", "20", "21", "22", "23", "24", "25", "26", "27", "28", "29", "30", "31", "32",
    "33", "34", "35", "36", "37", "38", "39", "40", "41", "42", "43", "44", "45", "46", "47", "48",
    "49", "50", "51", "52", "53", "54", "55", "56", "57", "58", "59", "60", "61", "62", "63",
];

struct History {
    rows: [Option<v8::Global<v8::Object>>; CAPACITY],
    oldest: usize,
    len: usize,
    local_seq: f64,
    host_cursor: Option<i32>,
    generation: u64,
}

impl Default for History {
    fn default() -> Self {
        Self {
            rows: std::array::from_fn(|_| None),
            oldest: 0,
            len: 0,
            local_seq: 0.0,
            host_cursor: None,
            generation: 0,
        }
    }
}

impl History {
    fn clear_messages(&mut self) {
        for row in &mut self.rows {
            *row = None;
        }
        self.oldest = 0;
        self.len = 0;
        self.local_seq = 0.0;
        self.generation = self.generation.wrapping_add(1);
    }

    fn clear_session(&mut self) {
        self.clear_messages();
        self.host_cursor = None;
    }

    fn push(&mut self, row: v8::Global<v8::Object>) {
        let index = if self.len < CAPACITY {
            let index = (self.oldest + self.len) % CAPACITY;
            self.len += 1;
            index
        } else {
            let index = self.oldest;
            self.rows[index] = None;
            self.oldest = (self.oldest + 1) % CAPACITY;
            index
        };
        self.rows[index] = Some(row);
    }
}

struct ChatboxState {
    history: RefCell<History>,
    unary_minus: v8::Global<v8::Function>,
}

/// Register the single private dispatcher and initialize isolate-owned state.
pub(crate) fn install(runtime: &mut Runtime) -> Result<(), String> {
    let unary_minus = {
        let mut scope = runtime.deno_runtime().handle_scope();
        let source = v8::String::new(&mut scope, NEGATE_SOURCE)
            .ok_or_else(|| "GameMessages unary operator source".to_string())?;
        let script = v8::Script::compile(&mut scope, source, None)
            .ok_or_else(|| "GameMessages unary operator compile".to_string())?;
        let value = script
            .run(&mut scope)
            .ok_or_else(|| "GameMessages unary operator run".to_string())?;
        let function = v8::Local::<v8::Function>::try_from(value)
            .map_err(|_| "GameMessages unary operator function".to_string())?;
        v8::Global::new(&mut scope, function)
    };
    let state = Rc::new(ChatboxState {
        history: RefCell::new(History::default()),
        unary_minus,
    });
    if !runtime.deno_runtime().v8_isolate().set_slot(state) {
        return Err("GameMessages state already installed".to_string());
    }
    cb::install(runtime, NAME, game_messages_callback)
}

/// Ingest the newly materialized posted page. The caller invokes this only
/// after a successful snapshot materialization that included chat rows.
pub(crate) fn ingest_posted(runtime: &mut Runtime) -> Result<(), String> {
    let mut scope = runtime.deno_runtime().handle_scope();
    let state = state_for_scope(&scope)
        .ok_or_else(|| "GameMessages native state is not installed".to_string())?;
    let page = current_lines(&mut scope).map_err(host_throw_error)?;
    let page = v8::Local::<v8::Array>::try_from(page)
        .map_err(|_| "GameMessages posted chat page is not an array".to_string())?;
    let mut cursor = state.history.borrow().host_cursor;
    let poll = cb::Poll::default();

    for index in (0..page.length()).rev() {
        poll.check(&mut scope).map_err(host_throw_error)?;
        let row = cb::get_index(&mut scope, page.into(), index).map_err(host_throw_error)?;
        let seq = cb::get(&mut scope, row, "seq").map_err(host_throw_error)?;
        if !seq.is_int32() {
            return Err("GameMessages posted chat sequence is not int32".to_string());
        }
        let host_seq = seq
            .int32_value(&mut scope)
            .ok_or_else(|| "GameMessages posted chat sequence is unavailable".to_string())?;
        if cursor.is_some_and(|seen| host_seq <= seen) {
            continue;
        }
        let text = cb::get(&mut scope, row, "text").map_err(host_throw_error)?;
        append_record(&mut scope, &state, text).map_err(host_throw_error)?;
        cursor = Some(host_seq);
        state.history.borrow_mut().host_cursor = cursor;
    }
    Ok(())
}

/// Clear history and the posted-host cursor at the ResetSession boundary.
pub(crate) fn reset_session(runtime: &mut Runtime) -> Result<(), String> {
    let state = runtime
        .deno_runtime()
        .v8_isolate()
        .get_slot::<Rc<ChatboxState>>()
        .cloned()
        .ok_or_else(|| "GameMessages native state is not installed".to_string())?;
    state.history.borrow_mut().clear_session();
    Ok(())
}

fn state_for_scope(scope: &v8::HandleScope) -> Option<Rc<ChatboxState>> {
    scope.get_slot::<Rc<ChatboxState>>().cloned()
}

fn game_messages_callback<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = dispatch(scope, args);
    cb::finish(scope, rv, result);
}

fn dispatch<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let opcode = args.get(0);
    if !opcode.is_int32() {
        return Err(cb::type_error(scope, "GameMessages opcode must be int32"));
    }
    let opcode = opcode
        .int32_value(scope)
        .ok_or_else(|| cb::type_error(scope, "GameMessages opcode must be int32"))?;
    match opcode {
        OP_RECORD => {
            let state = state_for_scope(scope)
                .ok_or_else(|| cb::type_error(scope, "GameMessages state is not installed"))?;
            append_record(scope, &state, args.get(1))?;
            Ok(v8::undefined(scope).into())
        }
        OP_MARK => mark(scope),
        OP_SINCE => since(scope, args.get(1)),
        OP_SAW_SINCE => saw_since(scope, args.get(1), args.get(2)),
        OP_FIRST_SINCE => first_since(scope, args.get(1), args.get(2)),
        OP_RECENT => recent(scope, args.get(1)),
        OP_RESET => {
            let state = state_for_scope(scope)
                .ok_or_else(|| cb::type_error(scope, "GameMessages state is not installed"))?;
            state.history.borrow_mut().clear_messages();
            Ok(v8::undefined(scope).into())
        }
        _ => Err(cb::type_error(scope, "unknown GameMessages opcode")),
    }
}

/// Read `globalThis.__rs2b0t_host || {}`, then its snapshot and chat page,
/// with the shim's short-circuiting truthiness and observable property reads.
fn current_lines<'s>(scope: &mut v8::HandleScope<'s>) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let global = scope.get_current_context().global(scope);
    let global_this = cb::get(scope, global.into(), "globalThis")?;
    let host = cb::get(scope, global_this, "__rs2b0t_host")?;
    let host = if cb::truthy(scope, host) {
        host
    } else {
        v8::Object::new(scope).into()
    };
    let snapshot = cb::get(scope, host, "snapshot")?;
    let snapshot = if cb::truthy(scope, snapshot) {
        snapshot
    } else {
        v8::Object::new(scope).into()
    };
    let lines = cb::get(scope, snapshot, "chat_lines")?;
    Ok(if cb::truthy(scope, lines) {
        lines
    } else {
        v8::Array::new(scope, 0).into()
    })
}

fn append_record<'s>(
    scope: &mut v8::HandleScope<'s>,
    state: &ChatboxState,
    text: v8::Local<'s, v8::Value>,
) -> JsResult<'s, ()> {
    let seq = {
        let mut history = state.history.borrow_mut();
        history.local_seq += 1.0;
        history.local_seq
    };
    let seq = v8::Number::new(scope, seq).into();
    let dto = cb::object(scope, &[("seq", seq), ("text", text)])?;
    let dto = v8::Local::<v8::Object>::try_from(dto)
        .map_err(|_| cb::type_error(scope, "could not create GameMessages record"))?;
    state.history.borrow_mut().push(v8::Global::new(scope, dto));
    Ok(())
}

fn mark<'s>(scope: &mut v8::HandleScope<'s>) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let lines = current_lines(scope)?;
    let length = cb::get(scope, lines, "length")?;
    let zero = v8::Integer::new(scope, 0);
    if length.strict_equals(zero.into()) {
        return Ok(zero.into());
    }
    let reduce = cb::get(scope, lines, "reduce")?;
    let callback = v8::Function::builder(mark_reduce_callback)
        .length(2)
        .build(scope)
        .ok_or_else(|| cb::type_error(scope, "could not create GameMessages reduce callback"))?;
    let initial = v8::Integer::new(scope, 0).into();
    cb::call(
        scope,
        reduce,
        lines,
        &[callback.into(), initial],
        "rs.reduce",
    )
}

fn since<'s>(
    scope: &mut v8::HandleScope<'s>,
    mark: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let lines = current_lines(scope)?;
    let filter = cb::get(scope, lines, "filter")?;
    let predicate = v8::Function::builder(page_since_callback)
        .data(mark)
        .length(1)
        .build(scope)
        .ok_or_else(|| cb::type_error(scope, "could not create GameMessages filter callback"))?;
    let filtered = cb::call(scope, filter, lines, &[predicate.into()], "lines.filter")?;
    let map = cb::get(scope, filtered, "map")?;
    let mapper = v8::Function::builder(page_dto_callback)
        .length(1)
        .build(scope)
        .ok_or_else(|| cb::type_error(scope, "could not create GameMessages map callback"))?;
    cb::call(scope, map, filtered, &[mapper.into()], "filtered.map")
}

fn saw_since<'s>(
    scope: &mut v8::HandleScope<'s>,
    mark: v8::Local<'s, v8::Value>,
    pattern: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let lines = current_lines(scope)?;
    let some = cb::get(scope, lines, "some")?;
    let data = pair_data(scope, mark, pattern)?;
    let predicate = v8::Function::builder(page_saw_callback)
        .data(data.into())
        .length(1)
        .build(scope)
        .ok_or_else(|| cb::type_error(scope, "could not create GameMessages some callback"))?;
    cb::call(scope, some, lines, &[predicate.into()], "lines.some")
}

fn first_since<'s>(
    scope: &mut v8::HandleScope<'s>,
    mark: v8::Local<'s, v8::Value>,
    pattern: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let state = state_for_scope(scope)
        .ok_or_else(|| cb::type_error(scope, "GameMessages state is not installed"))?;
    let (rows, generation) = history_view(scope, &state)?;
    let find = cb::get(scope, rows.into(), "find")?;
    let data = pair_data(scope, mark, pattern)?;
    let predicate = v8::Function::builder(history_find_callback)
        .data(data.into())
        .length(1)
        .build(scope)
        .ok_or_else(|| cb::type_error(scope, "could not create GameMessages find callback"))?;
    refresh_view_if_current(scope, &state, rows, generation)?;
    cb::call(scope, find, rows.into(), &[predicate.into()], "ring.find")
}

fn recent<'s>(
    scope: &mut v8::HandleScope<'s>,
    limit: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let state = state_for_scope(scope)
        .ok_or_else(|| cb::type_error(scope, "GameMessages state is not installed"))?;
    let zero = v8::Integer::new(scope, 0).into();
    if cb::le(scope, limit, zero)? {
        return Ok(v8::Array::new(scope, 0).into());
    }
    if state.history.borrow().len == 0 {
        return Ok(v8::Array::new(scope, 0).into());
    }

    let (rows, generation) = history_view(scope, &state)?;
    let slice = cb::get(scope, rows.into(), "slice")?;
    let unary_minus = v8::Local::new(scope, &state.unary_minus);
    let undefined = v8::undefined(scope).into();
    let negative_limit = cb::call(
        scope,
        unary_minus.into(),
        undefined,
        &[limit],
        "unary minus",
    )?;
    refresh_view_if_current(scope, &state, rows, generation)?;
    let sliced = cb::call(scope, slice, rows.into(), &[negative_limit], "ring.slice")?;
    cb::call_method(scope, sliced, "reverse", &[], "sliced.reverse")
}

fn history_view<'s>(
    scope: &mut v8::HandleScope<'s>,
    state: &ChatboxState,
) -> JsResult<'s, (v8::Local<'s, v8::Array>, u64)> {
    let history = state.history.borrow();
    let rows = v8::Array::new(scope, history.len as i32);
    for offset in 0..history.len {
        let index = (history.oldest + offset) % CAPACITY;
        let row = v8::Local::new(
            scope,
            history.rows[index]
                .as_ref()
                .expect("occupied GameMessages ring slot"),
        );
        set_array_data(scope, rows, offset, row.into())?;
    }
    Ok((rows, history.generation))
}

fn refresh_view_if_current<'s>(
    scope: &mut v8::HandleScope<'s>,
    state: &ChatboxState,
    rows: v8::Local<'s, v8::Array>,
    generation: u64,
) -> JsResult<'s, ()> {
    let history = state.history.borrow();
    if history.generation != generation {
        return Ok(());
    }
    let length = v8::String::new(scope, "length").unwrap_or_else(|| v8::String::empty(scope));
    let length_value = v8::Integer::new(scope, history.len as i32);
    let changed = cb::catching(scope, |scope| {
        rows.set(scope, length.into(), length_value.into())
    })?;
    if !changed {
        return Err(cb::type_error(
            scope,
            "could not refresh GameMessages history view",
        ));
    }
    for offset in 0..history.len {
        let index = (history.oldest + offset) % CAPACITY;
        let row = v8::Local::new(
            scope,
            history.rows[index]
                .as_ref()
                .expect("occupied GameMessages ring slot"),
        );
        set_array_data(scope, rows, offset, row.into())?;
    }
    Ok(())
}

fn set_array_data<'s>(
    scope: &mut v8::HandleScope<'s>,
    array: v8::Local<'s, v8::Array>,
    index: usize,
    value: v8::Local<'s, v8::Value>,
) -> JsResult<'s, ()> {
    let key =
        v8::String::new(scope, ARRAY_INDEXES[index]).unwrap_or_else(|| v8::String::empty(scope));
    let created = cb::catching(scope, |scope| {
        array.create_data_property(scope, key.into(), value)
    })?;
    if created {
        Ok(())
    } else {
        Err(cb::type_error(
            scope,
            "could not materialize GameMessages history",
        ))
    }
}

fn pair_data<'s>(
    scope: &mut v8::HandleScope<'s>,
    first: v8::Local<'s, v8::Value>,
    second: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Array>> {
    let data = v8::Array::new(scope, 2);
    set_array_data(scope, data, 0, first)?;
    set_array_data(scope, data, 1, second)?;
    Ok(data)
}

fn mark_reduce_callback<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = (|| {
        let global = scope.get_current_context().global(scope);
        let math = cb::get(scope, global.into(), "Math")?;
        let max = cb::get(scope, math, "max")?;
        let row = args.get(1);
        let seq_type = cb::get(scope, row, "seq")?;
        let seq = if seq_type.is_number() {
            cb::get(scope, row, "seq")?
        } else {
            cb::num(scope, 0.0)
        };
        cb::call(scope, max, math, &[args.get(0), seq], "Math.max")
    })();
    cb::finish(scope, rv, result);
}

fn page_since_callback<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = (|| {
        let row = args.get(0);
        let seq_type = cb::get(scope, row, "seq")?;
        if !seq_type.is_number() {
            return Ok(v8::Boolean::new(scope, false).into());
        }
        let seq = cb::get(scope, row, "seq")?;
        let newer = cb::gt(scope, seq, args.data())?;
        Ok(v8::Boolean::new(scope, newer).into())
    })();
    cb::finish(scope, rv, result);
}

fn page_dto_callback<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = (|| {
        let row = args.get(0);
        let seq = cb::get(scope, row, "seq")?;
        let text = cb::get(scope, row, "text")?;
        cb::object(scope, &[("seq", seq), ("text", text)])
    })();
    cb::finish(scope, rv, result);
}

fn page_saw_callback<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = match page_match(scope, args.get(0), args.data(), true) {
        Ok(value) => Ok(v8::Boolean::new(scope, value).into()),
        Err(error) => Err(error),
    };
    cb::finish(scope, rv, result);
}

fn history_find_callback<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = match page_match(scope, args.get(0), args.data(), false) {
        Ok(value) => Ok(v8::Boolean::new(scope, value).into()),
        Err(error) => Err(error),
    };
    cb::finish(scope, rv, result);
}

fn page_match<'s>(
    scope: &mut v8::HandleScope<'s>,
    row: v8::Local<'s, v8::Value>,
    data: v8::Local<'s, v8::Value>,
    numeric_only: bool,
) -> JsResult<'s, bool> {
    let seq_type = cb::get(scope, row, "seq")?;
    let seq = if numeric_only {
        if !seq_type.is_number() {
            return Ok(false);
        }
        cb::get(scope, row, "seq")?
    } else {
        seq_type
    };
    let mark = cb::get_index(scope, data, 0)?;
    if !cb::gt(scope, seq, mark)? {
        return Ok(false);
    }
    let text = cb::get(scope, row, "text")?;
    let pattern = cb::get_index(scope, data, 1)?;
    matches(scope, pattern, text)
}

fn matches<'s>(
    scope: &mut v8::HandleScope<'s>,
    pattern: v8::Local<'s, v8::Value>,
    text: v8::Local<'s, v8::Value>,
) -> JsResult<'s, bool> {
    let zero = v8::Integer::new(scope, 0).into();
    let last_index = cb::string(scope, "lastIndex");
    cb::assign(scope, pattern, last_index, zero)?;
    let test = cb::get(scope, pattern, "test")?;
    let result = cb::call(scope, test, pattern, &[text], "pattern.test")?;
    Ok(cb::truthy(scope, result))
}

fn host_throw_error<'s>(error: Throw<'s>) -> String {
    match error {
        Throw::Value(_) => "GameMessages posted chat access threw".to_string(),
        Throw::Terminated => "GameMessages posted chat access terminated".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustyscript::RuntimeOptions;

    const COMPLETE_BATCH: &str = r#"
        globalThis.__rs2b0t_host = { snapshot: { chat_lines: [
            { seq: 9, text: 'page newest' },
            { seq: 3, text: 'page oldest' },
        ] } };
        globalThis.__gameMessages = (...args) => __rs2b0t_game_messages(...args);
        globalThis.__completeChatboxBatch = () => {
            const gm = globalThis.__gameMessages;
            gm(6);
            gm(0, 'first history');
            gm(0, 'second history');
            const mark = gm(1);
            const page = gm(2, 3);
            const saw = gm(3, 0, /page newest/);
            const first = gm(4, 0, /first history/);
            const newest = gm(5, 1);
            const valid = mark === 9 &&
                page.length === 1 && page[0].seq === 9 &&
                first.seq === 1 && newest.length === 1 &&
                newest[0].seq === 2 && saw &&
                first !== newest[0] && page[0] !== globalThis.__rs2b0t_host.snapshot.chat_lines[0];
            gm(6);
            return valid;
        };
    "#;

    fn runtime() -> Runtime {
        crate::load::platform::bootstrap_test_v8_platform();
        let mut runtime = Runtime::new(RuntimeOptions::default()).unwrap();
        cb::install_ops(&mut runtime).unwrap();
        install(&mut runtime).unwrap();
        runtime.eval::<()>(COMPLETE_BATCH).unwrap();
        runtime
    }

    fn named_function<'s>(
        scope: &mut v8::HandleScope<'s>,
        name: &str,
    ) -> v8::Local<'s, v8::Function> {
        let global = scope.get_current_context().global(scope);
        let key = cb::string(scope, name);
        v8::Local::<v8::Function>::try_from(global.get(scope, key).unwrap()).unwrap()
    }

    fn assert_cold_bounded(info: allocation_counter::AllocationInfo) {
        const COLD_ALLOCATIONS: usize = 8;
        const COLD_BYTES: usize = COLD_ALLOCATIONS * 112;
        assert!(info.count_total <= COLD_ALLOCATIONS, "{info:?}");
        assert!(info.count_current <= COLD_ALLOCATIONS, "{info:?}");
        assert!(info.count_max <= COLD_ALLOCATIONS, "{info:?}");
        assert!(info.bytes_total <= COLD_BYTES, "{info:?}");
        assert!(info.bytes_current <= COLD_BYTES, "{info:?}");
        assert!(info.bytes_max <= COLD_BYTES, "{info:?}");
    }

    #[test]
    fn complete_chatbox_cold_batch_is_bounded_and_warm_rust_allocations_are_zero() {
        let mut runtime = runtime();
        let mut scope = runtime.deno_runtime().handle_scope();
        let function = named_function(&mut scope, "__completeChatboxBatch");
        let receiver = v8::undefined(&mut scope).into();
        let cold = allocation_counter::measure(|| {
            assert!(function.call(&mut scope, receiver, &[]).unwrap().is_true());
        });
        let warm = allocation_counter::measure(|| {
            for _ in 0..1_000 {
                assert!(function.call(&mut scope, receiver, &[]).unwrap().is_true());
            }
        });
        eprintln!("GameMessages complete cold={cold:?}; warm1000={warm:?}");
        assert_cold_bounded(cold);
        assert_eq!(warm, allocation_counter::AllocationInfo::default());
    }

    fn raw_scope_control(
        scope: &mut v8::HandleScope,
        args: v8::FunctionCallbackArguments,
        mut rv: v8::ReturnValue,
    ) {
        let first = &mut v8::TryCatch::new(scope);
        let second = &mut v8::TryCatch::new(first);
        let third = &mut v8::TryCatch::new(second);
        if args.get(0).is_true() {
            let fourth = &mut v8::TryCatch::new(third);
            rv.set(v8::Boolean::new(fourth, true).into());
        } else {
            rv.set(v8::Boolean::new(third, true).into());
        }
    }

    #[test]
    fn raw_scope_allocation_controls_use_an_independent_runtime() {
        crate::load::platform::bootstrap_test_v8_platform();
        let mut runtime = Runtime::new(RuntimeOptions::default()).unwrap();
        cb::install_ops(&mut runtime).unwrap();
        cb::install(&mut runtime, "__raw_scope_control", raw_scope_control).unwrap();
        runtime
            .eval::<()>(
                "globalThis.__rawThree = () => __raw_scope_control(false); \
                 globalThis.__rawFour = () => __raw_scope_control(true); \
                 globalThis.__rawPure = () => true;",
            )
            .unwrap();
        let mut scope = runtime.deno_runtime().handle_scope();
        let raw_three = named_function(&mut scope, "__rawThree");
        let raw_four = named_function(&mut scope, "__rawFour");
        let raw_pure = named_function(&mut scope, "__rawPure");
        let receiver = v8::undefined(&mut scope).into();
        let pure = allocation_counter::measure(|| {
            assert!(raw_pure.call(&mut scope, receiver, &[]).unwrap().is_true());
        });
        let cold = allocation_counter::measure(|| {
            assert!(raw_three.call(&mut scope, receiver, &[]).unwrap().is_true());
        });
        let deeper = allocation_counter::measure(|| {
            assert!(raw_four.call(&mut scope, receiver, &[]).unwrap().is_true());
        });
        let warm = allocation_counter::measure(|| {
            for _ in 0..1_000 {
                assert!(raw_three.call(&mut scope, receiver, &[]).unwrap().is_true());
                assert!(raw_four.call(&mut scope, receiver, &[]).unwrap().is_true());
            }
        });
        assert_eq!(pure, allocation_counter::AllocationInfo::default());
        assert_eq!(cold.count_total, 1);
        assert_eq!(cold.count_current, 1);
        assert_eq!(cold.count_max, 1);
        assert_eq!(cold.bytes_total, 112);
        assert_eq!(cold.bytes_current, 112);
        assert_eq!(cold.bytes_max, 112);
        assert_eq!(deeper.count_total, 1);
        assert_eq!(deeper.count_current, 1);
        assert_eq!(deeper.count_max, 1);
        assert_eq!(deeper.bytes_total, 112);
        assert_eq!(deeper.bytes_current, 112);
        assert_eq!(deeper.bytes_max, 112);
        assert_eq!(warm, allocation_counter::AllocationInfo::default());
    }

    #[test]
    fn host_cursor_deduplicates_and_reset_paths_have_distinct_cursor_policy() {
        let mut runtime = runtime();
        runtime
            .eval::<()>(
                "globalThis.__rs2b0t_host.snapshot.chat_lines = [\
                 { seq: 12, text: 'newest' }, { seq: 11, text: 'oldest' }];",
            )
            .unwrap();
        ingest_posted(&mut runtime).unwrap();
        ingest_posted(&mut runtime).unwrap();
        assert_eq!(
            runtime
                .eval::<serde_json::Value>("__rs2b0t_game_messages(5, 8).map(row => row.text)",)
                .unwrap(),
            serde_json::json!(["newest", "oldest"])
        );

        runtime.eval::<()>("__rs2b0t_game_messages(6)").unwrap();
        ingest_posted(&mut runtime).unwrap();
        assert_eq!(
            runtime
                .eval::<serde_json::Value>("__rs2b0t_game_messages(5, 8).map(row => row.text)",)
                .unwrap(),
            serde_json::json!([])
        );

        reset_session(&mut runtime).unwrap();
        ingest_posted(&mut runtime).unwrap();
        assert_eq!(
            runtime
                .eval::<serde_json::Value>("__rs2b0t_game_messages(5, 8).map(row => row.text)",)
                .unwrap(),
            serde_json::json!(["newest", "oldest"])
        );
    }

    #[test]
    fn ring_evicts_and_reset_releases_every_native_reference() {
        let mut runtime = runtime();
        runtime
            .eval::<()>("for (let i = 0; i < 130; i++) __rs2b0t_game_messages(0, 'line ' + i);")
            .unwrap();
        let state = runtime
            .deno_runtime()
            .v8_isolate()
            .get_slot::<Rc<ChatboxState>>()
            .cloned()
            .unwrap();
        {
            let history = state.history.borrow();
            assert_eq!(history.len, CAPACITY);
            assert_eq!(history.local_seq, 130.0);
            let oldest = history.rows[history.oldest].as_ref().unwrap();
            let mut scope = runtime.deno_runtime().handle_scope();
            let oldest = v8::Local::new(&mut scope, oldest);
            let text = cb::get(&mut scope, oldest.into(), "text").unwrap();
            assert_eq!(text.to_rust_string_lossy(&mut scope), "line 66");
        }
        runtime.eval::<()>("__rs2b0t_game_messages(6)").unwrap();
        let history = state.history.borrow();
        assert_eq!(history.len, 0);
        assert!(history.rows.iter().all(Option::is_none));
        assert_eq!(history.local_seq, 0.0);
    }
}
