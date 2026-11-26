//! The Luau sandbox through product runs (plan slice L3; ADR 0023 item 10;
//! rule C1 of `LUAU-DESIGN-1114/design-r3.md` 3.4, tests 2 and 3).
//!
//! Each test starts a real run from `LuauSurfaceFactory`, lets the script try
//! one kind of access, and then checks two things: every attempt raised a Luau
//! error, and the effect that the attempt would have had did not occur. The
//! census of the global table itself is in `src/runtime.rs` (test 1 of rule C1),
//! because a script has no `_G` with which to list its globals.

pub mod support;

use std::io::ErrorKind;
use std::net::TcpListener;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};

use script::surface::{LentOutcome, SurfaceRun, TickOutcome};
use support::{snapshot, spawn, tick};

/// The Luau code that runs each `{label, function}` probe in a guarded `pcall`
/// and logs `label<TAB>ok<TAB>result`. A probe that returns is a sandbox breach.
const PROBE_RUNNER: &str = r#"
local function run_probes(probes)
    for index = 1, #probes do
        local probe = probes[index]
        local ok, result = pcall(probe[2])
        api.log(probe[1] .. "\t" .. tostring(ok) .. "\t" .. tostring(result))
    end
end
"#;

/// A fresh directory under the target's temporary directory for this test.
fn scratch_dir(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("luau-sandbox-{name}-{}", std::process::id()));
    if dir.exists() {
        std::fs::remove_dir_all(&dir).expect("the old scratch directory is removable");
    }
    std::fs::create_dir_all(&dir).expect("the scratch directory is creatable");
    dir
}

/// A Luau string literal of `text`. Rust's escapes for quotes, backslashes and
/// control bytes are also Luau escapes.
fn luau_string(text: &str) -> String {
    format!("{text:?}")
}

/// Runs one tick of `source` and returns its log lines. The tick must end in
/// success: a probe error is caught, and nothing stops the run.
fn run_one_tick(source: &str) -> (Box<dyn SurfaceRun>, Vec<String>) {
    let mut run = spawn(source);
    assert_eq!(tick(&mut *run, &snapshot(1)), LentOutcome::Accepted);
    assert_eq!(
        run.drain_tick_outcomes(),
        [TickOutcome::Success {
            tick: 1,
            generation: 1
        }],
        "the probes must not end the tick"
    );
    let logs = run.drain_logs();
    (run, logs)
}

/// Runs `probes` (`label`, Luau expression) in one tick of a real run, after
/// the top-level statements of `header`, and checks that each probe raised. A
/// probe that returns is a sandbox breach. Returns the run and the error
/// message of each probe, in order.
fn raised_probes(header: &str, probes: &[(&str, String)]) -> (Box<dyn SurfaceRun>, Vec<String>) {
    let mut body = String::new();
    for (label, call) in probes {
        body.push_str(&format!(
            "    {{ {}, function() return {call} end }},\n",
            luau_string(label)
        ));
    }
    let source = format!(
        "{header}\n{PROBE_RUNNER}\nlocal probes = {{\n{body}}}\n\
         return {{ tick = function() run_probes(probes) end }}\n"
    );
    let (run, logs) = run_one_tick(&source);
    assert_eq!(
        logs.len(),
        probes.len(),
        "one log line for each probe: {logs:#?}"
    );
    let mut messages = Vec::with_capacity(logs.len());
    for (line, (label, _)) in logs.iter().zip(probes) {
        let mut fields = line.splitn(3, '\t');
        assert_eq!(fields.next(), Some(*label), "probe order: {line}");
        assert_eq!(
            fields.next(),
            Some("false"),
            "sandbox breach: probe `{label}` returned: {line}"
        );
        messages.push(fields.next().unwrap_or_default().to_owned());
    }
    (run, messages)
}

/// Each probe raised because the value it needs is missing, and not for
/// another reason.
fn assert_missing_values(probes: &[(&str, String)], messages: &[String]) {
    for ((label, _), message) in probes.iter().zip(messages) {
        assert!(
            message.contains("attempt to call a nil value")
                || message.contains("attempt to index nil"),
            "probe `{label}` failed for another reason than a missing value: {message}"
        );
    }
}

/// Runs `probes` in one tick of a real run and checks that each one raised
/// because the value it needs is missing. Returns the run and the messages.
fn assert_probes_refused(probes: &[(&str, String)]) -> (Box<dyn SurfaceRun>, Vec<String>) {
    let (run, messages) = raised_probes("", probes);
    assert_missing_values(probes, &messages);
    (run, messages)
}

#[test]
fn file_access_is_refused_and_leaves_the_file_system_unchanged() {
    let dir = scratch_dir("file");
    let secret = dir.join("secret.luau");
    let secret_text = "return \"luau-sandbox-secret-7f3a\"\n";
    std::fs::write(&secret, secret_text).expect("the secret file is writable");
    let created = dir.join("created.txt");
    let moved = dir.join("moved.luau");
    let secret_path = luau_string(&secret.to_string_lossy());
    let created_path = luau_string(&created.to_string_lossy());
    let moved_path = luau_string(&moved.to_string_lossy());
    let module_path = luau_string(&dir.join("secret").to_string_lossy());

    let probes = [
        (
            "io.open read",
            format!("io.open({secret_path}, 'r'):read('a')"),
        ),
        (
            "io.open write",
            format!("io.open({created_path}, 'w'):write('x')"),
        ),
        ("io.lines", format!("io.lines({secret_path})")),
        ("io.output", format!("io.output({created_path})")),
        ("io.read", "io.read('a')".to_owned()),
        ("dofile", format!("dofile({secret_path})")),
        ("loadfile", format!("loadfile({secret_path})()")),
        ("require path", format!("require({module_path})")),
        ("require relative", "require('./secret')".to_owned()),
        ("os.remove", format!("os.remove({secret_path})")),
        (
            "os.rename",
            format!("os.rename({secret_path}, {moved_path})"),
        ),
        ("os.tmpname", "os.tmpname()".to_owned()),
    ];
    let (_run, logs) = assert_probes_refused(&probes);
    assert!(
        logs.iter()
            .all(|line| !line.contains("luau-sandbox-secret")),
        "the script read the file: {logs:#?}"
    );
    assert_eq!(
        std::fs::read_to_string(&secret).expect("the secret file still exists"),
        secret_text,
        "the script changed the file"
    );
    let mut entries = std::fs::read_dir(&dir)
        .expect("the scratch directory is readable")
        .map(|entry| entry.expect("a directory entry").file_name())
        .collect::<Vec<_>>();
    entries.sort();
    assert_eq!(
        entries,
        ["secret.luau"],
        "the script created or moved a file"
    );
    std::fs::remove_dir_all(&dir).expect("the scratch directory is removable");
}

#[test]
fn os_access_is_refused_and_the_process_and_run_continue() {
    // ADR 0023 item 10: the crate never loads `os`, so a script has no clock,
    // date, environment, locale or process exit of its own.
    let dir = scratch_dir("os");
    let sentinel = dir.join("executed");
    let sentinel_path = luau_string(&sentinel.to_string_lossy());
    let command = luau_string(&format!("touch {}", sentinel.to_string_lossy()));
    let probes = [
        ("os.getenv", "os.getenv('PATH')".to_owned()),
        ("os.time", "os.time()".to_owned()),
        ("os.clock", "os.clock()".to_owned()),
        ("os.date", "os.date('%c')".to_owned()),
        ("os.difftime", "os.difftime(2, 1)".to_owned()),
        ("os.setlocale", "os.setlocale('C')".to_owned()),
        ("os.execute", format!("os.execute({command})")),
        ("os.remove", format!("os.remove({sentinel_path})")),
        ("os.exit", "os.exit(0)".to_owned()),
    ];
    let (mut run, logs) = assert_probes_refused(&probes);
    if let Some(path) = std::env::var_os("PATH").filter(|path| path.len() > 4) {
        let path = path.to_string_lossy();
        assert!(
            logs.iter().all(|line| !line.contains(path.as_ref())),
            "the script read the environment: {logs:#?}"
        );
    }
    assert!(!sentinel.exists(), "the script ran a command");
    // `os.exit` neither ended the test process nor the run: the next tick runs
    // the probes again.
    assert_eq!(tick(&mut *run, &snapshot(2)), LentOutcome::Accepted);
    assert_eq!(run.drain_logs().len(), probes.len());
    std::fs::remove_dir_all(&dir).expect("the scratch directory is removable");
}

#[test]
fn network_access_is_refused_and_no_connection_arrives() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback listener");
    listener
        .set_nonblocking(true)
        .expect("the listener is nonblocking");
    let port = listener.local_addr().expect("the listener address").port();
    let url = luau_string(&format!("http://127.0.0.1:{port}/"));
    let probes = [
        (
            "require socket",
            format!("require('socket').connect('127.0.0.1', {port})"),
        ),
        (
            "socket.connect",
            format!("socket.connect('127.0.0.1', {port})"),
        ),
        ("socket.tcp", "socket.tcp()".to_owned()),
        ("http.request", format!("http.request({url})")),
        ("http.get", format!("http.get({url})")),
        ("net.request", format!("net.request({url})")),
        ("request", format!("request({{ Url = {url} }})")),
        (
            "game HttpService",
            format!("game:GetService('HttpService'):GetAsync({url})"),
        ),
        (
            "package.loadlib",
            "package.loadlib('libc', 'connect')".to_owned(),
        ),
        ("io.popen curl", format!("io.popen('curl ' .. {url})")),
    ];
    assert_probes_refused(&probes);
    // A connect is synchronous: an attempt during the tick would already be in
    // the listener's backlog.
    match listener.accept() {
        Err(error) if error.kind() == ErrorKind::WouldBlock => {}
        Ok((_, peer)) => panic!("the script connected to the listener from {peer}"),
        Err(error) => panic!("the listener failed: {error}"),
    }
}

#[test]
fn process_and_code_loading_access_is_refused() {
    let dir = scratch_dir("process");
    let sentinel = dir.join("spawned");
    let command = luau_string(&format!("touch {}", sentinel.to_string_lossy()));
    let probes = [
        ("io.popen", format!("io.popen({command}):read('a')")),
        ("os.execute", format!("os.execute({command})")),
        ("require os", "require('os').execute('true')".to_owned()),
        (
            "package.loaded",
            "package.loaded.os.execute('true')".to_owned(),
        ),
        ("loadstring", "loadstring('return 1')()".to_owned()),
        ("load", "load('return 1')()".to_owned()),
        ("debug.getinfo", "debug.getinfo(1)".to_owned()),
        ("debug.getregistry", "debug.getregistry()".to_owned()),
        ("debug.getupvalue", "debug.getupvalue(print, 1)".to_owned()),
        ("debug.sethook", "debug.sethook()".to_owned()),
        ("getfenv", "getfenv(print)".to_owned()),
        ("setfenv", "setfenv(1, {})".to_owned()),
        ("coroutine.create", "coroutine.create(print)".to_owned()),
        ("coroutine.wrap", "coroutine.wrap(print)()".to_owned()),
        ("newproxy", "newproxy(true)".to_owned()),
        ("collectgarbage", "collectgarbage('count')".to_owned()),
        ("gcinfo", "gcinfo()".to_owned()),
        ("_G", "_G.os".to_owned()),
    ];
    assert_probes_refused(&probes);
    assert!(!sentinel.exists(), "the script started a process");
    std::fs::remove_dir_all(&dir).expect("the scratch directory is removable");
}

#[test]
fn removed_and_unloaded_names_fail_even_as_builtin_fast_calls() {
    // In a safe environment Luau runs a built-in function without a read of
    // the global table, so the compiler, not the global table, decides whether
    // a removed name still works. The compiler disables a built-in only for
    // the names that one source holds, so each probe runs alone: `unpack`
    // must fail in a source that never names `table`.
    let v = "vector.create(1, 2, 3)";
    let probes = [
        ("unpack", "unpack({1, 2, 3})".to_owned()),
        ("unpack range", "unpack({1, 2, 3}, 1, 3)".to_owned()),
        ("vector.create", v.to_owned()),
        ("vector.magnitude", format!("vector.magnitude({v})")),
        ("vector.normalize", format!("vector.normalize({v})")),
        ("vector.cross", format!("vector.cross({v}, {v})")),
        ("vector.dot", format!("vector.dot({v}, {v})")),
        ("vector.floor", format!("vector.floor({v})")),
        ("vector.ceil", format!("vector.ceil({v})")),
        ("vector.abs", format!("vector.abs({v})")),
        ("vector.sign", format!("vector.sign({v})")),
        ("vector.clamp", format!("vector.clamp({v}, {v}, {v})")),
        ("vector.min", format!("vector.min({v}, {v})")),
        ("vector.max", format!("vector.max({v}, {v})")),
        ("vector.lerp", format!("vector.lerp({v}, {v}, 0.5)")),
        ("integer.add", "integer.add(1, 2)".to_owned()),
        ("_VERSION", "_VERSION:len()".to_owned()),
        ("string.pack", "string.pack('i4', 1)".to_owned()),
        (
            "string.unpack",
            "string.unpack('i4', '\\0\\0\\0\\0')".to_owned(),
        ),
        ("string.packsize", "string.packsize('i4')".to_owned()),
        ("table.foreach", "table.foreach({1}, print)".to_owned()),
        ("table.foreachi", "table.foreachi({1}, print)".to_owned()),
    ];
    for probe in probes {
        assert_probes_refused(&[probe]);
    }
}

// The compiler skips a disabled dotted built-in `lib.fn` once the source
// writes a global named `fn`, but it still emits the fast call while `lib`
// itself is unwritten (review R1 of SLICE-L3-1125). Each case below writes the
// method name first; the fast call must stay off.

#[test]
fn written_method_names_do_not_reenable_off_list_fast_calls() {
    let v = "vector.create(1, 2, 3)";
    let cases = [
        (
            "create = false\ndot = false",
            ("vector.dot", format!("vector.dot({v}, {v})")),
        ),
        ("function create() end", ("vector.create", v.to_owned())),
        (
            "function create() end\nfunction abs() end",
            ("vector.abs", format!("vector.abs({v})")),
        ),
        (
            "create = 1\nmagnitude = 1",
            ("vector.magnitude", format!("vector.magnitude({v})")),
        ),
        (
            "local vec = vector\ncreate = 1",
            ("local alias", "vec.create(1, 2, 3)".to_owned()),
        ),
        ("add = 1", ("integer.add", "integer.add(1, 2)".to_owned())),
    ];
    assert_each_case(cases, |header, probe| {
        let probes = [probe];
        let (_run, messages) = raised_probes(header, &probes);
        assert_missing_values(&probes, &messages);
    });
}

#[test]
fn written_method_names_do_not_bypass_the_argument_count_guards() {
    // `math.max`, `math.min`, `bit32.band` and `bit32.bor` are disabled fast
    // calls because their work grows with the argument count; the prelude caps
    // that count at N_v (8000). A fast call would skip the cap.
    let many = "table.unpack(table.create(8001, 1))";
    let cases = [
        ("max = false", ("math.max", format!("math.max({many})"))),
        (
            "function min() end",
            ("math.min", format!("math.min({many})")),
        ),
        ("band = 1", ("bit32.band", format!("bit32.band({many})"))),
        (
            "local m = bit32\nbor = 1",
            ("local alias", format!("m.bor({many})")),
        ),
    ];
    assert_each_case(cases, |header, probe| {
        let label = probe.0;
        let (_run, messages) = raised_probes(header, &[probe]);
        assert!(
            messages[0].contains("argument count exceeds N_v (8000)"),
            "probe `{label}` after `{header}` did not reach the N_v guard: {messages:?}"
        );
    });
}

/// Runs `check` on every `(header, probe)` case and then fails with the
/// labels of all cases that failed, so one breach does not hide the others.
fn assert_each_case<const N: usize>(
    cases: [(&str, (&'static str, String)); N],
    check: impl Fn(&str, (&'static str, String)),
) {
    let failed = cases
        .into_iter()
        .filter_map(|(header, probe)| {
            let label = probe.0;
            catch_unwind(AssertUnwindSafe(|| check(header, probe)))
                .err()
                .map(|_| label)
        })
        .collect::<Vec<_>>();
    assert!(failed.is_empty(), "cases that failed: {failed:?}");
}

#[test]
fn libraries_api_and_metatables_are_read_only_across_entries() {
    // The first tick tries to replace or extend each shared table and the
    // string metatable; each write raises. The second tick shows that the
    // libraries and the API still work.
    let source = r#"
        local probes = {
            {"string field", function() string.rep = nil end},
            {"rawset string", function() rawset(string, "rep", print) end},
            {"setmetatable string", function() setmetatable(string, {}) end},
            {"math field", function() math.floor = nil end},
            {"table field", function() table.sort = nil end},
            {"bit32 field", function() bit32.band = nil end},
            {"utf8 field", function() utf8.char = nil end},
            {"buffer field", function() buffer.create = nil end},
            {"string metatable", function() getmetatable("").__index = {} end},
            {"rawset string metatable", function() rawset(getmetatable(""), "__index", {}) end},
            {"api member", function() api.log = print end},
            {"rawset api", function() rawset(api, "log", nil) end},
            {"api group", function() api.settings.str = print end},
            {"rawset api group", function() rawset(api.settings, "extra", print) end},
            {"setmetatable api", function() setmetatable(api, {}) end},
        }
        local ticks = 0
        return { tick = function()
            ticks = ticks + 1
            if ticks == 1 then
                for index = 1, #probes do
                    local ok, message = pcall(probes[index][2])
                    api.log(probes[index][1] .. "\t" .. tostring(ok) .. "\t" .. tostring(message))
                end
            else
                api.log(
                    string.rep("ab", 2),
                    math.floor(2.5),
                    table.concat({"x", "y"}),
                    getmetatable("").__index == string,
                    getmetatable(api) == nil,
                    api.settings.extra == nil
                )
            end
        end }
    "#;
    let (mut run, logs) = run_one_tick(source);
    assert_eq!(logs.len(), 15, "{logs:#?}");
    for line in &logs {
        let mut fields = line.splitn(3, '\t');
        let label = fields.next().unwrap_or_default();
        assert_eq!(fields.next(), Some("false"), "write `{label}` succeeded");
        assert!(
            fields
                .next()
                .is_some_and(|message| message.contains("readonly")),
            "write `{label}` failed for another reason: {line}"
        );
    }
    assert_eq!(tick(&mut *run, &snapshot(2)), LentOutcome::Accepted);
    assert_eq!(run.drain_logs(), ["abab\t2\txy\ttrue\ttrue\ttrue"]);
}
