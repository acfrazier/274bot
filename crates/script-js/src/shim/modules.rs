use std::{borrow::Cow, collections::VecDeque, path::Path, sync::LazyLock};

use deno_ast::swc::{
    ast::{CallExpr, Callee, Expr, Lit, ModuleDecl, ModuleItem},
    ecma_visit::{Visit, VisitWith},
};
use deno_ast::{MediaType, ParseParams, ProgramRef};
use rustyscript::module_loader::ImportProvider;
use rustyscript::Module;

/// Canonical path of the user's bot module. Fixed for every card so
/// relative rs2b0t imports (`../../api/...`) resolve to the same shim
/// URLs no matter which script loaded; it is a synthetic specifier only
/// (nothing is read from `/rs2b0t` on this machine).
pub const BOT_MODULE: &str = "/rs2b0t/bot/scripts/bot/bot.js";
/// Canonical path of the shape wrapper that imports `./bot.js`.
pub const MAIN_MODULE: &str = "/rs2b0t/bot/scripts/bot/main.js";

/// The prelude eval'd into every isolate before any module loads: host state,
/// bot base classes, `defineBot`, and the recording canvas ctx for `onPaint`.
/// TaskBot sequencing stays in the prelude; TreeBot delegates its traversal
/// to a native binding. Abstract task classes live in `bot.js`.
pub const PRELUDE: &str = r#"
globalThis.__rs2b0t_host = {};
globalThis.__rs2b0t_channel_next = 1;
globalThis.__rs2b0t_channels = new Map();
globalThis.BroadcastChannel = class BroadcastChannel {
    constructor(name) {
        this.name = String(name);
        this.onmessage = null;
        this.onmessageerror = null;
        this._closed = false;
        this._listeners = { message: [], messageerror: [] };
        this._id = globalThis.__rs2b0t_channel_next++;
        globalThis.__rs2b0t_channels.set(this._id, this);
        const h = globalThis.__rs2b0t_host;
        const interact = h.interact ?? (h.interact = []);
        interact.push({ op: 'channel-open', channel_id: this._id, name: this.name });
    }
    postMessage(value) {
        if (this._closed) throw new Error('BroadcastChannel is closed');
        const data = globalThis.rustyscript.functions.__rs2b0t_channel_encode(value);
        const h = globalThis.__rs2b0t_host;
        const interact = h.interact ?? (h.interact = []);
        interact.push({ op: 'channel-post', channel_id: this._id, name: this.name, data });
    }
    close() {
        if (this._closed) return;
        this._closed = true;
        globalThis.__rs2b0t_channels.delete(this._id);
        const h = globalThis.__rs2b0t_host;
        const interact = h.interact ?? (h.interact = []);
        interact.push({ op: 'channel-close', channel_id: this._id, name: this.name });
    }
    addEventListener(type, callback) {
        if ((type === 'message' || type === 'messageerror') && typeof callback === 'function') {
            this._listeners[type].push(callback);
        }
    }
    removeEventListener(type, callback) {
        const rows = this._listeners[type];
        if (!rows) return;
        const at = rows.indexOf(callback);
        if (at >= 0) rows.splice(at, 1);
    }
    dispatchEvent(event) {
        const type = event && event.type;
        if (type !== 'message' && type !== 'messageerror') return false;
        const property = this['on' + type];
        if (typeof property === 'function') property.call(this, event);
        for (const callback of this._listeners[type].slice()) callback.call(this, event);
        return true;
    }
};
// The isolate thread holds broker deliveries and calls one of these per
// delivery inside the next tick; a refusal is logged there too.
globalThis.__rs2b0t_channel_deliver = (id, data, sender) => {
    const channel = globalThis.__rs2b0t_channels.get(id);
    if (!channel || channel._closed) return;
    channel.dispatchEvent({
        type: 'message', data, origin: '', lastEventId: '', source: null, ports: [],
        sender: String(sender || ''),
    });
};
globalThis.__rs2b0t_channel_refused = (id, error) => {
    const channel = globalThis.__rs2b0t_channels.get(id);
    if (!channel || channel._closed) return;
    channel.dispatchEvent({ type: 'messageerror', data: null, error });
};
globalThis.__rs2b0t_event_interrupt = null;
globalThis.__rs2b0t_interrupt_pending = () => {
    const callback = globalThis.__rs2b0t_event_interrupt;
    if (typeof callback !== 'function') return false;
    try { return callback() === true; } catch (_) { return false; }
};
// Monotonic isolate clock (rustyscript's default extensions have no
// `performance`): elapsed ms since the isolate thread started, from the
// host-registered `__rs2b0t_now`. Execution delay/delayUntil use it.
globalThis.performance = {
    now: () => globalThis.rustyscript.functions.__rs2b0t_now(),
};
globalThis.defineBot = (manifest) => {
    if (!manifest || typeof manifest.name !== 'string' || manifest.name.length === 0 || typeof manifest.create !== 'function') {
        throw new Error('defineBot requires { name, create }');
    }
    return { __rs2b0tManifest: 1, ...manifest };
};
globalThis.LoopingBot = class LoopingBot {
            loopDelay = 600;
            loopCadence = null;
            subscriptions = [];
            onStart() {}
            onStop() {}
            onPause() {}
            onResume() {}
            onPaint() {}
            loop() {}
            recoveryAnchor() { return null; }
            grindTargets() { return []; }
            ignoredRandoms() { return []; }
            on(event, cb) {
                if (typeof cb !== 'function') return;
                const id = globalThis.__rs2b0t_native_event_on(event, cb);
                this.subscriptions.push(() => globalThis.__rs2b0t_native_event_off(id));
            }
    bindLog(sink) {
        this._logSink = typeof sink === 'function' ? sink : null;
    }
    log(message) {
        if (this._logSink) {
            this._logSink(String(message));
            return;
        }
        const h = globalThis.__rs2b0t_host;
        h.log = h.log || [];
        h.log.push(String(message));
    }
    // Frozen AbstractBot finish: a bound sink (a delegating parent)
    // hears the reason; otherwise only requestFinish marks the clean host
    // stop that emits script.finish. ScriptRunner.stop remains silent.
    requestFinish(reason) {
        if (this._finishSink) {
            this._finishSink(reason);
            return;
        }
        const h = globalThis.__rs2b0t_host;
        h.stopRequested = true;
        h.requestFinishRequested = true;
        if (typeof reason === 'string') h.stopReason = reason;
    }
    bindFinish(sink) {
        this._finishSink = typeof sink === 'function' ? sink : null;
    }
    get delegated() { return !!this._finishSink; }
    disposeSubscriptions() {
        for (const unsubscribe of this.subscriptions || []) unsubscribe();
        this.subscriptions = [];
    }
    get settings() {
        if (this._settings !== undefined) return this._settings;
        const bag = globalThis.__rs2b0t_host.settingsBag || {};
        // view is rebuilt only when the host posts a new bag.
        const cached = globalThis.__rs2b0t_settings_view;
        if (cached !== undefined && cached.bag === bag) return cached.view;
        const view = {
            str(name, fallback = '') {
                const v = bag[name];
                return typeof v === 'string' ? v : fallback;
            },
            num(name, fallback = 0) {
                const v = bag[name];
                if (typeof v === 'number' && !Number.isNaN(v)) return v;
                if (typeof v === 'string' && v !== '' && !Number.isNaN(Number(v))) return Number(v);
                return fallback;
            },
            bool(name, fallback = false) {
                const v = bag[name];
                if (typeof v === 'boolean') return v;
                if (v === 'true') return true;
                if (v === 'false') return false;
                return fallback;
            },
            tile(name, fallback = null) {
                const v = bag[name];
                const fromPosted = globalThis.__rs2b0t_tileFromPosted;
                return typeof fromPosted === 'function' ? (fromPosted(v) ?? fallback) : fallback;
            },
            list(name, fallback = []) {
                const v = bag[name];
                return Array.isArray(v) ? v : fallback;
            },
        };
        globalThis.__rs2b0t_settings_view = { bag, view };
        return view;
    }
    set settings(value) {
        this._settings = value;
    }
};
globalThis.__rs2b0t_dispatch_native_events = (events) => {
    globalThis.__rs2b0t_native_event_batch_dispatch(events);
};
globalThis.__rs2b0t_invoke_native_event_listener = (callback, payload, hasPayload) => {
    const logError = (err) => {
        const h = globalThis.__rs2b0t_host;
        if (!h) return;
        h.log = h.log || [];
        const message = String((err && err.message) || err).slice(0, 256);
        h.log.push(`[rs2b0t] listener error: ${message}`);
    };
    try {
        const result = hasPayload ? callback(payload) : callback();
        if (result && typeof result.then === 'function') {
            result.then(() => {}, logError);
        }
    } catch (err) {
        logError(err);
    }
};
globalThis.TaskBot = class TaskBot extends globalThis.LoopingBot {
    constructor() {
        super();
        this._tasks = [];
        this.activeTaskName = null;
    }
    add(...tasks) {
        this._tasks.push(...tasks);
    }
    async loop() {
        for (const task of this._tasks) {
            if (await task.validate()) {
                this.activeTaskName = task.label ?? null;
                await task.execute();
                this.activeTaskName = null;
                return;
            }
        }
        this.activeTaskName = null;
    }
};
globalThis.TreeBot = class TreeBot extends globalThis.LoopingBot {
    lastSceneWaitLogAt = 0;
    async loop() {
        await globalThis.__rs2b0t_tree_loop(this);
    }
};
// Paint ctx: records its calls into a Float64Array op tape plus a
// deduplicated string table and flushes them in one typed crossing. Op codes
// and arity mirror `load/canvas_tape.rs`. `measureText` needs recorder state,
// so it flushes first; style getters answer from the ctx's own state and never
// cross.
globalThis.__rs2b0t_make_paint_ctx = () => {
    const fn = globalThis.rustyscript.functions;
    fn.__rs2b0t_canvas_begin();
    const SET = 1, SET_NUM = 2, FILL_GRADIENT = 3, FILL_RECT = 4, FILL_TEXT = 5,
        SAVE = 6, RESTORE = 7, BEGIN_PATH = 8, CLOSE_PATH = 9, MOVE_TO = 10,
        LINE_TO = 11, QUAD_TO = 12, ARC = 13, FILL = 14, STROKE = 15, CLIP = 16,
        CREATE_LINEAR = 17, CREATE_RADIAL = 18, ADD_STOP = 19;
    let cap = 256;
    let tape = new Float64Array(cap);
    let n = 0;
    const strs = [];
    const ids = new Map();
    const put = (v) => {
        if (n === cap) {
            cap *= 2;
            const grown = new Float64Array(cap);
            grown.set(tape);
            tape = grown;
        }
        tape[n++] = v;
    };
    const s = (v) => {
        const key = String(v);
        const seen = ids.get(key);
        if (seen !== undefined) return seen;
        const i = strs.length;
        strs.push(key);
        ids.set(key, i);
        return i;
    };
    // json_f64 parity: only a finite number crosses; anything else is 0.
    const num = (v) => (typeof v === 'number' && Number.isFinite(v) ? v : 0);
    const flush = () => {
        if (n === 0) return;
        const ops = tape.subarray(0, n);
        n = 0;
        const err = globalThis.__rs2b0t_canvas_submit(ops, strs);
        // The ops in flight reference this table; the next flush sends its own.
        strs.length = 0;
        ids.clear();
        if (typeof err === 'string') throw new Error(err);
    };
    const grads = Object.create(null);
    let gradCount = 0;
    const gradObj = (id) => {
        if (!grads[id]) {
            const g = {
                addColorStop(offset, color) {
                    const at = num(Number(offset));
                    // Same reason as `arc`: the declared ctx throws this at the
                    // call site, not at the flush.
                    if (at < 0 || at > 1) {
                        throw new Error('IndexSizeError: offset must be in [0, 1]');
                    }
                    put(ADD_STOP);
                    put(id);
                    put(at);
                    put(s(color));
                }
            };
            Object.defineProperty(g, '__rs2b0t_gradient', { value: id });
            grads[id] = g;
        }
        return grads[id];
    };
    // Getters read the recorder, so a value the recorder rejects reads back as
    // the previous one, as a canvas does. Pending sets land first, which is
    // what the per-call path did by construction.
    const ctx = {
        set font(v) { put(SET); put(s('font')); put(s(v)); },
        get font() { flush(); return fn.__rs2b0t_canvas_get('font'); },
        set fillStyle(v) {
            if (v && typeof v === 'object' && typeof v.__rs2b0t_gradient === 'number') {
                put(FILL_GRADIENT);
                put(v.__rs2b0t_gradient);
                return;
            }
            put(SET);
            put(s('fillStyle'));
            put(s(v));
        },
        get fillStyle() {
            flush();
            const id = fn.__rs2b0t_canvas_fill_gradient_id();
            if (id >= 0) return gradObj(id);
            return fn.__rs2b0t_canvas_get('fillStyle');
        },
        set strokeStyle(v) { put(SET); put(s('strokeStyle')); put(s(v)); },
        get strokeStyle() { flush(); return fn.__rs2b0t_canvas_get('strokeStyle'); },
        set shadowColor(v) { put(SET); put(s('shadowColor')); put(s(v)); },
        get shadowColor() { flush(); return fn.__rs2b0t_canvas_get('shadowColor'); },
        set lineJoin(v) { put(SET); put(s('lineJoin')); put(s(v)); },
        get lineJoin() { flush(); return fn.__rs2b0t_canvas_get('lineJoin'); },
        set textAlign(v) { put(SET); put(s('textAlign')); put(s(v)); },
        get textAlign() { flush(); return fn.__rs2b0t_canvas_get('textAlign'); },
        set textBaseline(v) { put(SET); put(s('textBaseline')); put(s(v)); },
        get textBaseline() { flush(); return fn.__rs2b0t_canvas_get('textBaseline'); },
        set lineWidth(v) { put(SET_NUM); put(s('lineWidth')); put(num(Number(v))); },
        get lineWidth() { flush(); return fn.__rs2b0t_canvas_get_num('lineWidth'); },
        set shadowBlur(v) { put(SET_NUM); put(s('shadowBlur')); put(num(Number(v))); },
        get shadowBlur() { flush(); return fn.__rs2b0t_canvas_get_num('shadowBlur'); },
        set shadowOffsetX(v) { put(SET_NUM); put(s('shadowOffsetX')); put(num(Number(v))); },
        get shadowOffsetX() { flush(); return fn.__rs2b0t_canvas_get_num('shadowOffsetX'); },
        set shadowOffsetY(v) { put(SET_NUM); put(s('shadowOffsetY')); put(num(Number(v))); },
        get shadowOffsetY() { flush(); return fn.__rs2b0t_canvas_get_num('shadowOffsetY'); },
        fillRect(x, y, w, h) { put(FILL_RECT); put(num(x)); put(num(y)); put(num(w)); put(num(h)); },
        fillText(text, x, y) {
            if (arguments.length >= 4) throw new Error('not impl: Canvas.fillText.maxWidth');
            put(FILL_TEXT);
            put(s(text));
            put(num(x));
            put(num(y));
        },
        measureText(text) {
            flush();
            return { width: fn.__rs2b0t_canvas_measure_text(String(text)) };
        },
        save() { put(SAVE); },
        restore() { put(RESTORE); },
        beginPath() { put(BEGIN_PATH); },
        closePath() { put(CLOSE_PATH); },
        moveTo(x, y) { put(MOVE_TO); put(num(x)); put(num(y)); },
        lineTo(x, y) { put(LINE_TO); put(num(x)); put(num(y)); },
        quadraticCurveTo(cpx, cpy, x, y) {
            put(QUAD_TO);
            put(num(cpx));
            put(num(cpy));
            put(num(x));
            put(num(y));
        },
        arc(x, y, r, a0, a1, ccw) {
            const radius = num(r);
            // The recorder's IndexSizeError is part of the declared ctx, so it
            // stays a call-site throw instead of surfacing at the flush.
            if (radius < 0) throw new Error('IndexSizeError: radius must be non-negative');
            put(ARC);
            put(num(x));
            put(num(y));
            put(radius);
            put(num(a0));
            put(num(a1));
            put(ccw ? 1 : 0);
        },
        fill() { put(FILL); },
        stroke() { put(STROKE); },
        clip() { put(CLIP); },
        createLinearGradient(x0, y0, x1, y1) {
            put(CREATE_LINEAR);
            put(num(x0));
            put(num(y0));
            put(num(x1));
            put(num(y1));
            return gradObj(gradCount++);
        },
        createRadialGradient(x0, y0, r0, x1, y1, r1) {
            const inner = num(r0);
            const outer = num(r1);
            // Same call-site contract as `arc`.
            if (inner < 0 || outer < 0) {
                throw new Error('IndexSizeError: radii must be non-negative');
            }
            put(CREATE_RADIAL);
            put(num(x0));
            put(num(y0));
            put(inner);
            put(num(x1));
            put(num(y1));
            put(outer);
            return gradObj(gradCount++);
        },
    };
    const styleProps = {
        font: 1, fillStyle: 1, strokeStyle: 1, shadowColor: 1, lineJoin: 1,
        textAlign: 1, textBaseline: 1, lineWidth: 1, shadowBlur: 1,
        shadowOffsetX: 1, shadowOffsetY: 1
    };
    const guarded = new Proxy(ctx, {
        get(target, prop) {
            if (typeof prop === 'symbol') return target[prop];
            if (prop === 'canvas') return undefined;
            if (prop in target) return target[prop];
            throw new Error('not impl: Canvas.' + String(prop));
        },
        set(target, prop, value) {
            if (typeof prop === 'string' && styleProps[prop]) {
                target[prop] = value;
                return true;
            }
            throw new Error('not impl: Canvas.' + String(prop));
        },
        has(target, prop) {
            if (prop === 'canvas') return false;
            return prop in target;
        },
    });
    return { ctx: guarded, flush };
};
globalThis.__rs2b0t_call_on_paint = (bot) => {
    bot = bot || globalThis.__rs_bot;
    const h = globalThis.__rs2b0t_host;
    const fn = globalThis.rustyscript.functions;
    if (!bot || typeof bot.onPaint !== 'function') {
        fn.__rs2b0t_canvas_begin();
        fn.__rs2b0t_canvas_onpaint_done(1);
        return;
    }
    const paint = globalThis.__rs2b0t_make_paint_ctx();
    try {
        bot.onPaint(paint.ctx);
        paint.flush();
        fn.__rs2b0t_canvas_onpaint_done(0);
    } catch (e) {
        // Ops recorded before the throw land, as the per-call path did; a
        // failed flush already applies every op before the failing one and
        // clears the tape, so this retry never re-applies them.
        try { paint.flush(); } catch (_) {}
        const msg = String((e && e.message) || e);
        h.lastError = msg;
        fn.__rs2b0t_canvas_onpaint_done(2, msg);
    }
};
globalThis.KeyboardEvent = function KeyboardEvent(type, init) {
    init = init || {};
    this.type = String(type || '');
    this.key = String(init.key || '');
    this.code = String(init.code || '');
};
globalThis.MouseEvent = function MouseEvent(type, init) {
    init = init || {};
    this.type = String(type || '');
    this.clientX = Object.prototype.hasOwnProperty.call(init, 'clientX') ? init.clientX : 0;
    this.clientY = Object.prototype.hasOwnProperty.call(init, 'clientY') ? init.clientY : 0;
    this.button = Object.prototype.hasOwnProperty.call(init, 'button') ? init.button : 0;
};
globalThis.__rs2b0t_input_canvas = {
    getBoundingClientRect() {
        const r = globalThis.__rs2b0t_host && globalThis.__rs2b0t_host.canvasRect;
        if (!r) throw new Error('BLOCKED: missing getBoundingClientRect');
        return r;
    },
    dispatchEvent(ev) {
        if (ev && (ev.type === 'keydown' || ev.type === 'keyup')) {
            const h = globalThis.__rs2b0t_host;
            const interact = h.interact ?? (h.interact = []);
            interact.push({
                op: 'key',
                down: ev.type === 'keydown',
                key: String(ev.key || ''),
                code: String(ev.code || ''),
            });
            return true;
        }
        if (ev && (ev.type === 'mousedown' || ev.type === 'mouseup')) {
            const h = globalThis.__rs2b0t_host;
            const interact = h.interact ?? (h.interact = []);
            interact.push({
                op: 'mouse',
                down: ev.type === 'mousedown',
                x: ev.clientX,
                y: ev.clientY,
                button: ev.button,
            });
            return true;
        }
        throw new Error('BLOCKED: missing mouse');
    },
};
globalThis.document = {
    getElementById(id) {
        return id === 'canvas' ? globalThis.__rs2b0t_input_canvas : null;
    },
};
"#;

/// The extra modules that make catalog imports hit our shim, in load order
/// (a module must be registered before anything imports it; the bot's own
/// module is appended by the caller, after these). Most paths mirror `src/bot/`;
/// the only cross-root projections are `src/client/config/worlds.ts` and
/// `src/client/shell/Skill.ts`, which AccountLeveler imports. No other client
/// modules are served.
#[cfg(test)]
const DECLARED_SURFACE_MODULE: &str = "/rs2b0t/bot/scripts/bot/declared_surface.js";
#[cfg(test)]
const RS2B0T_API_MODULE: &str = "/rs2b0t/bot/scripts/bot/rs2b0t-api.js";
#[cfg(test)]
const WALK_DESTINATIONS_MODULE: &str = "/rs2b0t/bot/api/map/WalkDestinations.js";
#[cfg(test)]
const REGIONS_MODULE: &str = "/rs2b0t/bot/api/map/regions.js";
#[cfg(test)]
const WILDERNESS_MODULE: &str = "/rs2b0t/bot/event/webwalk/wilderness.js";
#[cfg(test)]
const WALK_EXECUTOR_MODULE: &str = "/rs2b0t/bot/event/webwalk/WalkExecutor.js";
#[cfg(test)]
const STOCK_MODEL_MODULE: &str = "/rs2b0t/bot/api/shop/StockModel.js";
#[cfg(test)]
const SHOP_PRICE_MODULE: &str = "/rs2b0t/bot/api/shop/shopPrice.js";
#[cfg(test)]
const WAVE_THREE_MODULE_PATHS: [&str; 6] = [
    WALK_DESTINATIONS_MODULE,
    REGIONS_MODULE,
    WILDERNESS_MODULE,
    WALK_EXECUTOR_MODULE,
    STOCK_MODEL_MODULE,
    SHOP_PRICE_MODULE,
];
const EAGER_MODULE_PATHS: [&str; 4] = [
    // Isolate and lifecycle code call Execution globals without an import.
    "/rs2b0t/bot/api/execution/Execution.js",
    // The prelude settings getter reads this helper through globalThis.
    "/rs2b0t/bot/geometry/Tile.js",
    // Native TreeBot code reads the BranchTask constructor through globalThis.
    "/rs2b0t/bot/api/bot/Bot.js",
    // Scheduler reads this context view through globalThis.
    "/rs2b0t/bot/runtime/ScriptRunner.js",
];

pub fn shim_modules() -> Vec<Module> {
    super::source_catalog::SHIM_MODULE_SOURCES
        .iter()
        .map(|(filename, source)| Module::new(*filename, *source))
        .collect()
}

#[derive(Default)]
struct ModuleEdges {
    imports: Vec<usize>,
    dynamic_imports: Vec<usize>,
    has_unknown_dependency: bool,
    has_dynamic_code: bool,
}

#[derive(Default)]
struct ShimImportGraph {
    edges: Vec<ModuleEdges>,
    paths: Vec<std::path::PathBuf>,
    gated: Vec<bool>,
    complete: bool,
}

fn shim_import_graph() -> &'static ShimImportGraph {
    static GRAPH: LazyLock<ShimImportGraph> = LazyLock::new(build_shim_import_graph);
    &GRAPH
}

fn build_shim_import_graph() -> ShimImportGraph {
    let modules = shim_modules();
    let mut graph = ShimImportGraph {
        paths: modules
            .iter()
            .map(|module| module.filename().to_path_buf())
            .collect(),
        gated: modules
            .iter()
            .map(|module| !is_eager_module(module.filename()))
            .collect(),
        complete: true,
        ..ShimImportGraph::default()
    };
    for module in &modules {
        let Some(url) = module_specifier(module.filename()) else {
            graph.complete = false;
            continue;
        };
        let Some(edges) = module_edges(module.contents(), &url, &modules) else {
            graph.complete = false;
            continue;
        };
        graph.edges.push(edges);
    }
    graph
}

fn is_eager_module(path: &Path) -> bool {
    EAGER_MODULE_PATHS
        .iter()
        .any(|eager| Path::new(eager) == path)
}

fn module_specifier(path: &Path) -> Option<deno_ast::ModuleSpecifier> {
    let path = rustyscript::deno_core::normalize_path(Cow::Borrowed(path));
    rustyscript::deno_core::url::Url::from_file_path(path.as_ref()).ok()
}

fn module_edges(
    source: &str,
    referrer: &deno_ast::ModuleSpecifier,
    modules: &[Module],
) -> Option<ModuleEdges> {
    let parsed = deno_ast::parse_module(ParseParams {
        specifier: referrer.clone(),
        text: source.into(),
        media_type: MediaType::JavaScript,
        capture_tokens: false,
        scope_analysis: false,
        maybe_syntax: None,
    })
    .ok()?;
    if !parsed.diagnostics().is_empty() {
        return None;
    }
    let ProgramRef::Module(module) = parsed.program_ref() else {
        return None;
    };

    let mut edges = ModuleEdges::default();
    for item in &module.body {
        let ModuleItem::ModuleDecl(decl) = item else {
            continue;
        };
        match decl {
            ModuleDecl::Import(import) => add_import_edge(
                &mut edges,
                import.src.value.as_ref(),
                false,
                referrer,
                modules,
            ),
            ModuleDecl::ExportAll(export) => add_import_edge(
                &mut edges,
                export.src.value.as_ref(),
                false,
                referrer,
                modules,
            ),
            ModuleDecl::ExportNamed(export) => {
                if let Some(specifier) = export.src.as_ref() {
                    add_import_edge(
                        &mut edges,
                        specifier.value.as_ref(),
                        false,
                        referrer,
                        modules,
                    );
                }
            }
            _ => {}
        }
    }

    let mut visitor = DynamicDependencyVisitor::default();
    module.visit_with(&mut visitor);
    edges.has_unknown_dependency |= visitor.has_unknown_dependency;
    edges.has_dynamic_code = visitor.has_dynamic_code;
    for specifier in visitor.dynamic_imports {
        add_import_edge(&mut edges, &specifier, true, referrer, modules);
    }
    Some(edges)
}

#[derive(Default)]
struct DynamicDependencyVisitor {
    dynamic_imports: Vec<String>,
    has_unknown_dependency: bool,
    has_dynamic_code: bool,
}

impl Visit for DynamicDependencyVisitor {
    fn visit_call_expr(&mut self, call: &CallExpr) {
        match &call.callee {
            Callee::Import(_) => match call
                .args
                .first()
                .filter(|argument| argument.spread.is_none())
                .map(|argument| &*argument.expr)
            {
                Some(Expr::Lit(Lit::Str(specifier))) => {
                    self.dynamic_imports.push(specifier.value.to_string());
                }
                Some(Expr::Tpl(template)) if template.exprs.is_empty() => {
                    match template.quasis.as_slice() {
                        [quasi] => match &quasi.cooked {
                            Some(specifier) => self.dynamic_imports.push(specifier.to_string()),
                            None => self.has_unknown_dependency = true,
                        },
                        _ => self.has_unknown_dependency = true,
                    }
                }
                _ => self.has_unknown_dependency = true,
            },
            Callee::Expr(callee)
                if is_identifier(callee, "eval") || is_identifier(callee, "Function") =>
            {
                // Keep the existing direct-call fallback as a fast path.
                // The import provider below handles every served module that
                // dynamic code reaches but this narrow scan cannot see.
                self.has_dynamic_code = true;
            }
            _ => {}
        }
        call.visit_children_with(self);
    }

    fn visit_new_expr(&mut self, expression: &deno_ast::swc::ast::NewExpr) {
        if is_identifier(&expression.callee, "Function") {
            self.has_dynamic_code = true;
        }
        expression.visit_children_with(self);
    }
}

fn is_identifier(expression: &Expr, expected: &str) -> bool {
    match expression {
        Expr::Ident(identifier) => identifier.sym.as_ref() == expected,
        Expr::Paren(paren) => is_identifier(&paren.expr, expected),
        _ => false,
    }
}

fn add_import_edge(
    edges: &mut ModuleEdges,
    specifier: &str,
    dynamic: bool,
    referrer: &deno_ast::ModuleSpecifier,
    modules: &[Module],
) {
    match rustyscript::deno_core::resolve_import(specifier, referrer.as_str()) {
        Ok(url) => match url.to_file_path() {
            Ok(path) => match modules
                .iter()
                .position(|module| module.filename() == path.as_path())
            {
                Some(index) => {
                    if dynamic {
                        edges.dynamic_imports.push(index);
                    } else {
                        edges.imports.push(index);
                    }
                }
                None => edges.has_unknown_dependency = true,
            },
            Err(_) => edges.has_unknown_dependency = true,
        },
        Err(_) => edges.has_unknown_dependency = true,
    }
}

fn enqueue_imports(edges: &ModuleEdges, pending: &mut VecDeque<usize>) -> Result<(), &'static str> {
    if edges.has_dynamic_code {
        return Err("dynamic code construction");
    }
    if edges.has_unknown_dependency {
        return Err("unresolved module import");
    }
    pending.extend(edges.imports.iter().chain(&edges.dynamic_imports).copied());
    Ok(())
}

fn log_import_gate_disabled(card_name: &str, reason: &str) {
    let card_name = card_name.replace(['\r', '\n'], " ");
    api::host_log!(
        api::hostlog::Category::ScriptLifecycle,
        api::hostlog::Level::Warn,
        slot = card_name.as_str(),
        "import-closure gating disabled: {reason}"
    );
}

pub(crate) fn filter_gated_modules_for_card<'a>(
    modules: &mut Vec<Module>,
    roots: impl IntoIterator<Item = &'a str>,
    card_name: &str,
) {
    let graph = shim_import_graph();
    if !graph.complete
        || graph.paths.len() > modules.len()
        || graph.gated.len() != graph.paths.len()
        || graph.edges.len() != graph.paths.len()
    {
        log_import_gate_disabled(card_name, "incomplete shim import graph");
        return;
    }
    if graph
        .paths
        .iter()
        .zip(modules.iter())
        .any(|(path, module)| module.filename() != path.as_path())
    {
        log_import_gate_disabled(card_name, "served module set differs from graph");
        return;
    }

    let mut pending = VecDeque::with_capacity(modules.len());
    for root in roots {
        let Some(index) = modules
            .iter()
            .position(|module| module.filename() == Path::new(root))
        else {
            log_import_gate_disabled(card_name, "root module is missing");
            return;
        };
        pending.push_back(index);
    }
    // load_modules evaluates every supplied side module. Include all modules
    // that remain eager as roots, as well as the script and its siblings.
    pending.extend(
        graph
            .gated
            .iter()
            .enumerate()
            .filter_map(|(index, &is_gated)| (!is_gated).then_some(index)),
    );

    let mut visited = vec![false; modules.len()];
    let mut needed = vec![false; modules.len()];
    while let Some(index) = pending.pop_front() {
        if visited[index] {
            continue;
        }
        visited[index] = true;
        if graph.gated.get(index).copied().unwrap_or(false) {
            needed[index] = true;
        }
        let edges = if let Some(edges) = graph.edges.get(index) {
            edges
        } else {
            let module = &modules[index];
            let Some(referrer) = module_specifier(module.filename()) else {
                log_import_gate_disabled(card_name, "module URL could not be resolved");
                return;
            };
            let Some(edges) = module_edges(module.contents(), &referrer, modules) else {
                log_import_gate_disabled(card_name, "module parse failed");
                return;
            };
            if let Err(reason) = enqueue_imports(&edges, &mut pending) {
                log_import_gate_disabled(card_name, reason);
                return;
            }
            continue;
        };
        if let Err(reason) = enqueue_imports(edges, &mut pending) {
            log_import_gate_disabled(card_name, reason);
            return;
        }
    }

    let mut index = 0;
    modules.retain(|_| {
        let current_index = index;
        index += 1;
        !graph.gated.get(current_index).copied().unwrap_or(false) || needed[current_index]
    });
}

#[cfg(test)]
fn filter_gated_modules<'a>(modules: &mut Vec<Module>, roots: impl IntoIterator<Item = &'a str>) {
    filter_gated_modules_for_card(modules, roots, "test card");
}

#[cfg(feature = "test-hooks")]
#[doc(hidden)]
pub fn filter_gated_modules_for_test<'a>(
    modules: &mut Vec<Module>,
    roots: impl IntoIterator<Item = &'a str>,
) {
    filter_gated_modules_for_card(modules, roots, "test card");
}

#[cfg(feature = "test-hooks")]
#[doc(hidden)]
pub fn filter_gated_modules_for_card_test<'a>(
    modules: &mut Vec<Module>,
    roots: impl IntoIterator<Item = &'a str>,
    card_name: &str,
) {
    filter_gated_modules_for_card(modules, roots, card_name);
}

fn shim_source(specifier: &rustyscript::deno_core::ModuleSpecifier) -> Option<&'static str> {
    let path = specifier.to_file_path().ok()?;
    super::source_catalog::SHIM_MODULE_SOURCES
        .iter()
        .find_map(|(served_path, source)| {
            (Path::new(served_path) == path.as_path()).then_some(*source)
        })
}

#[derive(Default)]
struct ShimImportProvider;

impl ImportProvider for ShimImportProvider {
    fn resolve(
        &mut self,
        specifier: &rustyscript::deno_core::ModuleSpecifier,
        _referrer: &str,
        _kind: rustyscript::deno_core::ResolutionKind,
    ) -> Option<
        Result<
            rustyscript::deno_core::ModuleSpecifier,
            rustyscript::deno_core::error::ModuleLoaderError,
        >,
    > {
        shim_source(specifier)?;
        Some(Ok(specifier.clone()))
    }

    fn import(
        &mut self,
        specifier: &rustyscript::deno_core::ModuleSpecifier,
        _referrer: Option<&rustyscript::deno_core::ModuleSpecifier>,
        _is_dyn_import: bool,
        _requested_module_type: rustyscript::deno_core::RequestedModuleType,
    ) -> Option<Result<String, rustyscript::deno_core::error::ModuleLoaderError>> {
        shim_source(specifier).map(|source| Ok(source.to_owned()))
    }
}

pub(crate) fn shim_import_provider() -> Box<dyn ImportProvider> {
    Box::<ShimImportProvider>::default()
}

/// Rewrite bare `@rs2b0t/api` import specifiers (quoted) to
/// `./rs2b0t-api.js`, which resolves to our bundle module. rustyscript
/// 0.12's loader cannot resolve bare specifiers — `resolve_import` fails
/// before any import provider runs — so the source is remapped instead of
/// an import map. Only exact quoted `@rs2b0t/api` specifiers are touched.
pub(crate) fn remap_rs2b0t_api(source: &str) -> String {
    const BARE: &str = "@rs2b0t/api";
    let mut out = String::with_capacity(source.len());
    let mut rest = source;
    while let Some(idx) = rest.find(BARE) {
        let before = rest[..idx].chars().next_back();
        let after = rest[idx + BARE.len()..].chars().next();
        out.push_str(&rest[..idx]);
        if matches!(before, Some('\'' | '"')) && matches!(after, Some('\'' | '"')) {
            out.push_str("./rs2b0t-api.js");
        } else {
            out.push_str(BARE);
        }
        rest = &rest[idx + BARE.len()..];
    }
    out.push_str(rest);
    out
}

/// Rewrite quoted `#/bot/` import prefixes to the absolute shim URLs
/// rustyscript can resolve (`/rs2b0t/bot/...`). The unloadable scan already
/// treats hash paths as remapped; Start still has to rewrite the source or
/// the loader rejects the specifier as a relative path.
pub(crate) fn remap_hash_bot(source: &str) -> String {
    const FROM: &str = "#/bot/";
    const TO: &str = "/rs2b0t/bot/";
    let mut out = String::with_capacity(source.len());
    let mut rest = source;
    while let Some(idx) = rest.find(FROM) {
        let before = rest[..idx].chars().next_back();
        out.push_str(&rest[..idx]);
        if matches!(before, Some('\'' | '"')) {
            out.push_str(TO);
        } else {
            out.push_str(FROM);
        }
        rest = &rest[idx + FROM.len()..];
    }
    out.push_str(rest);
    out
}

/// Catalog import rewrites that must happen before rustyscript resolves
/// the bot (and any scripts-folder siblings).
pub fn remap_catalog_imports(source: &str) -> String {
    remap_hash_bot(&remap_rs2b0t_api(source))
}
#[cfg(test)]
mod wave_three_tests {
    use super::*;

    const THIEVER_IMPORTS: &str = r#"import '../../api/combat/food.js';
import '../../api/tasks/Anchor.js';
import '../../api/tasks/ContinueDialog.js';
import '../../api/bot/Bot.js';
import '../../api/execution/Execution.js';
import '../../api/game/Game.js';
import '../../event/webwalk/geometry/Reachability.js';
import '../../geometry/Tile.js';
import '../../api/ui/dialogue/ChatDialog.js';
import '../../api/bank/Bank.js';
import '../../api/bank/Banking.js';
import '../../api/bank/BankLocations.js';
import '../../event/webwalk/walkOpening.js';
import '../../api/inventory/Inventory.js';
import '../../api/skills/Skills.js';
import '../../api/grounditems/GroundItems.js';
import '../../api/npcs/Npcs.js';
import '../../paint/Paint.js';
import '../../paint/paintLogic.js';
import '../../runtime/ScriptRunner.js';
import '../../runtime/Settings.js';
import '../../api/walking/Traversal.js';
import '../../data/pickpocketTargets.js';
import '../../api/thieving/targets.js';
import '../../api/thieving/stealRules.js';
import '../../api/loadout/loadoutPlan.js';
import '../../api/loadout/loadoutSetting.js';
export default class T {}"#;

    fn gated_module_paths() -> Vec<String> {
        shim_modules()
            .iter()
            .filter(|module| !is_eager_module(module.filename()))
            .map(|module| module.filename().display().to_string())
            .collect()
    }

    fn baseline_gated_paths() -> Vec<String> {
        retained_gated("export default class T {}")
    }

    fn retained_gated(source: &str) -> Vec<String> {
        let gated_paths = gated_module_paths();
        let mut modules = shim_modules();
        modules.push(Module::new(BOT_MODULE, source));
        filter_gated_modules(&mut modules, std::iter::once(BOT_MODULE));
        gated_paths
            .into_iter()
            .filter(|path| {
                modules
                    .iter()
                    .any(|module| module.filename() == Path::new(path))
            })
            .collect()
    }

    fn retained_wave_three(source: &str) -> Vec<String> {
        retained_gated(source)
            .into_iter()
            .filter(|path| WAVE_THREE_MODULE_PATHS.contains(&path.as_str()))
            .collect()
    }

    #[test]
    fn wave_three_modules_follow_static_import_and_reexport_closures() {
        assert!(retained_wave_three("export default class T {}").is_empty());
        assert_eq!(
            retained_wave_three("import '../../api/map/WalkDestinations.js';"),
            [WALK_DESTINATIONS_MODULE]
        );
        assert_eq!(
            retained_wave_three("export { WALK_DESTINATIONS } from './declared_surface.js';"),
            [WALK_DESTINATIONS_MODULE]
        );
    }

    #[test]
    fn dynamic_imports_keep_their_wave_three_targets() {
        assert_eq!(
            retained_wave_three(
                "export async function load() { return import('../../api/map/WalkDestinations.js'); }"
            ),
            [WALK_DESTINATIONS_MODULE]
        );
        assert_eq!(
            retained_gated("export async function load(specifier) { return import(specifier); }"),
            gated_module_paths()
        );
    }

    #[test]
    fn javascript_parser_keeps_all_reviewed_regex_and_import_cases() {
        let cases: &[(&str, &str)] = &[
            (
                "regex-backtick-static",
                r#"const clean = (s) => s.replace(/`/g, '');
import * as regions from '../../api/map/regions.js';
// one backtick: `"#,
            ),
            (
                "regex-backtick-dynamic",
                r#"const clean = (s) => s.replace(/`/g, '');
const mod = await import('../../api/map/regions.js');
// `"#,
            ),
            (
                "regex-comment-dynamic",
                r#"const split = (s) => s.split(/[/*]/);
const mod = await import('../../api/map/regions.js');
/** doc */"#,
            ),
            (
                "regex-backslash",
                r#"const clean = (s) => s.replace(/\s+/g, ' ');
const mod = await import('../../api/map/regions.js');"#,
            ),
            (
                "regex-optchain-backtick",
                r#"const c = (s) => s?.replace(/`/g, '');
await import('../../api/map/regions.js');
// `"#,
            ),
            (
                "regex-after-if-paren",
                r#"if (true) /`/.test('x');
await import('../../api/map/regions.js');
// `"#,
            ),
            (
                "regex-quote",
                r#"const q = (s) => s.replace(/'/g, '');
await import('../../api/map/regions.js');
// '"#,
            ),
            (
                "regex-class-slash-star",
                r#"const r = /[*/]+/;
await import('../../api/map/regions.js');
/* x */"#,
            ),
            (
                "template-substitution-import",
                r#"const t = `${(await import('../../api/map/regions.js')) ? 'y' : 'n'}`;"#,
            ),
            (
                "template-nested-template",
                r#"const t = `a${`b${(await import('../../api/map/regions.js')) ? 1 : 0}`}`;"#,
            ),
            (
                "nested-function-dynamic",
                r#"async function f() { return import('../../api/map/regions.js'); }
await f();"#,
            ),
            (
                "class-static-block",
                r#"class Q { static { globalThis.__p = import('../../api/map/regions.js'); } }
await globalThis.__p;"#,
            ),
            (
                "comments-inside-import-call",
                r#"await import(/* ` */ '../../api/map/regions.js' /* */);"#,
            ),
            (
                "no-subst-template",
                r#"await import(`../../api/map/regions.js`);"#,
            ),
            (
                "escaped-specifier",
                r#"await import('\u002e./../api/map/regions.js');"#,
            ),
            (
                "export-star-from",
                r#"export * from '../../api/map/regions.js';"#,
            ),
            (
                "export-star-as",
                r#"export * as R from '../../api/map/regions.js';"#,
            ),
            (
                "string-then-import",
                r#"const s = "import('x') /* `";
await import('../../api/map/regions.js');"#,
            ),
            (
                "comment-backtick-then-import",
                "// ` /*
await import('../../api/map/regions.js');",
            ),
            (
                "hashbang",
                "#!/usr/bin/env node
await import('../../api/map/regions.js');",
            ),
            (
                "trailing-comma-arg",
                r#"await import('../../api/map/regions.js',);"#,
            ),
            (
                "javascript-typeof-import-comparison",
                r#"const f = 1, x = 2;
globalThis.__q = f < typeof import('../../api/map/regions.js') > (x);"#,
            ),
        ];
        for (name, source) in cases {
            assert_eq!(
                retained_wave_three(source),
                [REGIONS_MODULE],
                "parser failed case {name}"
            );
        }
    }

    #[test]
    fn import_text_in_non_code_does_not_create_edges() {
        let baseline = baseline_gated_paths();
        for source in [
            "const s = \"import('../../api/map/regions.js')\";",
            "const s = `import('../../api/map/regions.js')`;",
            "// import '../../api/map/regions.js';\n/* await import('../../api/map/regions.js') */",
            r"const r = /import\('..\/..\/api\/map\/regions.js'\)/;",
        ] {
            assert_eq!(
                retained_gated(source),
                baseline,
                "text without an import edge changed the baseline closure: {source}"
            );
        }
    }

    #[test]
    fn unresolved_imports_and_existing_direct_code_fallback_keep_all_gated_modules() {
        for source in [
            "export default class {",
            "await import('../../api/map/__nope.js').catch(() => 0);",
            "const specifier = '../../api/map/' + 'regions.js'; await import(specifier);",
            "import type { Regions } from '../../api/map/regions.js';",
            "import { type Regions } from '../../api/map/regions.js';",
            "export type { Regions } from '../../api/map/regions.js';",
            "await eval(\"import('/rs2b0t/bot/api/map/regions.js')\");",
            "await new Function(\"return import('/rs2b0t/bot/api/map/regions.js')\")();",
        ] {
            assert_eq!(
                retained_gated(source),
                gated_module_paths(),
                "fallback did not retain the gated set for {source}"
            );
        }
    }

    #[test]
    fn escaped_dynamic_code_is_not_an_eager_gate_fallback() {
        let baseline = baseline_gated_paths();
        let sources = [
            "await (0, eval)(\"import('/rs2b0t/bot/api/map/regions.js')\");",
            "await globalThis.eval(\"import('/rs2b0t/bot/api/map/regions.js')\");",
            "const e = eval; await e(\"import('/rs2b0t/bot/api/map/regions.js')\");",
            "await eval?.(\"import('/rs2b0t/bot/api/map/regions.js')\");",
            "await Function`return import('/rs2b0t/bot/api/map/regions.js')`();",
            "await new globalThis.Function(\"return import('/rs2b0t/bot/api/map/regions.js')\")();",
            "await Function.call(null, \"return import('/rs2b0t/bot/api/map/regions.js')\")();",
            "await Reflect.construct(Function, [\"return import('/rs2b0t/bot/api/map/regions.js')\"])();",
            "await (async () => {}).constructor(\"return import('/rs2b0t/bot/api/map/regions.js')\")();",
            "await Function?.(\"return import('/rs2b0t/bot/api/map/regions.js')\")();",
        ];
        for source in sources {
            assert_eq!(
                retained_gated(source),
                baseline,
                "escaped syntax changed the eager closure: {source}"
            );
        }
    }

    #[test]
    fn source_catalog_matches_every_registered_shim() {
        let modules = shim_modules();
        let sources = super::super::source_catalog::SHIM_MODULE_SOURCES;
        assert_eq!(modules.len(), sources.len());
        for (module, (path, source)) in modules.iter().zip(sources) {
            assert_eq!(module.filename(), Path::new(path));
            assert_eq!(module.contents(), *source);
            let specifier = module_specifier(Path::new(path)).expect("absolute shim URL");
            assert_eq!(shim_source(&specifier), Some(*source));
        }
    }

    #[test]
    fn wave_three_modules_follow_sibling_import_roots() {
        const SIBLING_MODULE: &str = "/rs2b0t/bot/scripts/bot/sibling.js";
        let mut modules = shim_modules();
        modules.push(Module::new(
            SIBLING_MODULE,
            "import '../../api/map/regions.js';",
        ));
        modules.push(Module::new(BOT_MODULE, "export default class T {}"));
        filter_gated_modules(&mut modules, [BOT_MODULE, SIBLING_MODULE]);
        assert_eq!(
            WAVE_THREE_MODULE_PATHS
                .iter()
                .filter(|path| {
                    modules
                        .iter()
                        .any(|module| module.filename() == Path::new(path))
                })
                .copied()
                .collect::<Vec<_>>(),
            [REGIONS_MODULE]
        );
    }

    #[test]
    fn unimported_modules_keep_global_hooks_and_drop_unreferenced_shims() {
        let mut modules = shim_modules();
        modules.push(Module::new(BOT_MODULE, "export default class T {}"));
        filter_gated_modules(&mut modules, std::iter::once(BOT_MODULE));

        let mut retained_shims = modules
            .iter()
            .filter_map(|module| {
                let path = module.filename().to_str()?;
                (path.starts_with("/rs2b0t/bot/") && path != BOT_MODULE).then_some(path)
            })
            .collect::<Vec<_>>();
        retained_shims.sort_unstable();
        assert_eq!(
            retained_shims,
            [
                "/rs2b0t/bot/api/bot/Bot.js",
                "/rs2b0t/bot/api/execution/Execution.js",
                "/rs2b0t/bot/geometry/Tile.js",
                "/rs2b0t/bot/runtime/ScriptRunner.js",
                "/rs2b0t/bot/shim/_kernel.js",
            ]
        );
    }

    #[test]
    fn every_served_shim_is_gated_except_documented_eager_roots() {
        let modules = shim_modules();
        assert_eq!(EAGER_MODULE_PATHS.len(), 4);
        for eager in EAGER_MODULE_PATHS {
            assert!(
                modules
                    .iter()
                    .any(|module| module.filename() == Path::new(eager)),
                "documented eager module is not served: {eager}"
            );
        }
        assert_eq!(
            gated_module_paths().len(),
            modules.len() - EAGER_MODULE_PATHS.len()
        );
    }

    #[test]
    fn thiever_keeps_direct_imports_but_not_public_or_unrelated_catalogs() {
        let retained = retained_gated(THIEVER_IMPORTS);
        for path in [
            "/rs2b0t/bot/api/combat/food.js",
            "/rs2b0t/bot/api/tasks/Anchor.js",
            "/rs2b0t/bot/api/game/Game.js",
            "/rs2b0t/bot/event/webwalk/geometry/Reachability.js",
            "/rs2b0t/bot/api/ui/dialogue/ChatDialog.js",
            "/rs2b0t/bot/api/bank/Bank.js",
            "/rs2b0t/bot/api/bank/Banking.js",
            "/rs2b0t/bot/api/bank/BankLocations.js",
            "/rs2b0t/bot/event/webwalk/walkOpening.js",
            "/rs2b0t/bot/api/inventory/Inventory.js",
            "/rs2b0t/bot/api/skills/Skills.js",
            "/rs2b0t/bot/api/grounditems/GroundItems.js",
            "/rs2b0t/bot/api/npcs/Npcs.js",
            "/rs2b0t/bot/paint/Paint.js",
            "/rs2b0t/bot/paint/paintLogic.js",
            "/rs2b0t/bot/runtime/Settings.js",
            "/rs2b0t/bot/api/walking/Traversal.js",
            "/rs2b0t/bot/data/pickpocketTargets.js",
            "/rs2b0t/bot/api/thieving/targets.js",
            "/rs2b0t/bot/api/thieving/stealRules.js",
            "/rs2b0t/bot/api/loadout/loadoutPlan.js",
            "/rs2b0t/bot/api/loadout/loadoutSetting.js",
        ] {
            assert!(
                retained.iter().any(|retained| retained == path),
                "Thiever import closure omitted {path}"
            );
        }
        for path in [
            DECLARED_SURFACE_MODULE,
            RS2B0T_API_MODULE,
            WALK_DESTINATIONS_MODULE,
            REGIONS_MODULE,
            WILDERNESS_MODULE,
            WALK_EXECUTOR_MODULE,
            STOCK_MODEL_MODULE,
            SHOP_PRICE_MODULE,
        ] {
            assert!(
                retained.iter().all(|retained| retained != path),
                "Thiever import closure retained unrelated module {path}"
            );
        }
        let mut sorted_retained = retained;
        sorted_retained.extend(EAGER_MODULE_PATHS.iter().map(|path| (*path).to_string()));
        sorted_retained.sort_unstable();
        assert_eq!(
            sorted_retained,
            [
                "/rs2b0t/bot/adapter/ClientAdapter.js",
                "/rs2b0t/bot/api/bank/Bank.js",
                "/rs2b0t/bot/api/bank/BankLocations.js",
                "/rs2b0t/bot/api/bank/Banking.js",
                "/rs2b0t/bot/api/bot/Bot.js",
                "/rs2b0t/bot/api/combat/food.js",
                "/rs2b0t/bot/api/execution/Execution.js",
                "/rs2b0t/bot/api/game/Game.js",
                "/rs2b0t/bot/api/grounditems/GroundItems.js",
                "/rs2b0t/bot/api/inventory/Inventory.js",
                "/rs2b0t/bot/api/loadout/loadoutPlan.js",
                "/rs2b0t/bot/api/loadout/loadoutSetting.js",
                "/rs2b0t/bot/api/npcs/Npcs.js",
                "/rs2b0t/bot/api/query/Query.js",
                "/rs2b0t/bot/api/skills/Skills.js",
                "/rs2b0t/bot/api/sustain/Sustain.js",
                "/rs2b0t/bot/api/tasks/Anchor.js",
                "/rs2b0t/bot/api/tasks/ContinueDialog.js",
                "/rs2b0t/bot/api/thieving/stealRules.js",
                "/rs2b0t/bot/api/thieving/targets.js",
                "/rs2b0t/bot/api/ui/dialogue/ChatDialog.js",
                "/rs2b0t/bot/api/walking/Traversal.js",
                "/rs2b0t/bot/data/pickpocketTargets.js",
                "/rs2b0t/bot/event/webwalk/geometry/Reachability.js",
                "/rs2b0t/bot/event/webwalk/walkOpening.js",
                "/rs2b0t/bot/geometry/Tile.js",
                "/rs2b0t/bot/paint/Paint.js",
                "/rs2b0t/bot/paint/paintLogic.js",
                "/rs2b0t/bot/runtime/ScriptRunner.js",
                "/rs2b0t/bot/runtime/Settings.js",
                "/rs2b0t/bot/shim/_kernel.js",
            ]
        );
    }

    #[test]
    fn importing_public_api_barrel_retains_its_module_closure() {
        let source = remap_catalog_imports("import '@rs2b0t/api'; export default class T {}");
        let retained = retained_gated(&source);
        for path in [
            DECLARED_SURFACE_MODULE,
            RS2B0T_API_MODULE,
            "/rs2b0t/bot/api/bank/Bank.js",
            WALK_DESTINATIONS_MODULE,
        ] {
            assert!(
                retained.iter().any(|retained| retained == path),
                "public API barrel closure omitted {path}"
            );
        }
    }

    #[test]
    fn literal_template_imports_keep_their_wave_three_targets() {
        assert_eq!(
            retained_wave_three(
                "export async function load() { return import(`../../api/map/regions.js`); }"
            ),
            [REGIONS_MODULE]
        );
    }

    #[test]
    fn comments_strings_and_templates_do_not_create_import_edges() {
        let source = r#"const text = "import('../../api/map/regions.js') eval('x') new Function('x')";
// import '../../api/map/regions.js'; eval('x'); new Function('x');
const template = `import('../../api/map/regions.js') eval("x") new Function("x")`;
export default class T {}"#;
        assert_eq!(retained_gated(source), baseline_gated_paths());
    }

    #[test]
    fn regex_literals_do_not_hide_following_static_imports() {
        let source = r#"
const pattern = /`\/\* import\(['"]Regions/;
import '../../api/map/regions.js';
export default class T {}
"#;
        assert_eq!(retained_wave_three(source), [REGIONS_MODULE]);
    }

    #[test]
    fn dynamic_code_construction_keeps_all_gated_modules() {
        for source in [
            "const dynamicImport = new Function('u', 'return import(u)');",
            r#"eval("import('/rs2b0t/bot/api/map/regions.js')");"#,
            r#"Function("return import('/rs2b0t/bot/api/map/regions.js')")();"#,
        ] {
            assert_eq!(retained_gated(source), gated_module_paths());
        }
    }

    #[test]
    fn dynamic_code_in_a_sibling_keeps_all_gated_modules() {
        const SIBLING_MODULE: &str = "/rs2b0t/bot/scripts/bot/sibling.js";
        let mut modules = shim_modules();
        modules.push(Module::new(
            SIBLING_MODULE,
            "const dynamicImport = new Function('u', 'return import(u)');",
        ));
        modules.push(Module::new(BOT_MODULE, "export default class T {}"));
        filter_gated_modules(&mut modules, [BOT_MODULE, SIBLING_MODULE]);
        let gated_paths = gated_module_paths();
        let retained = gated_paths
            .iter()
            .filter(|path| {
                modules
                    .iter()
                    .any(|module| module.filename() == Path::new(path))
            })
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(retained, gated_paths);
    }

    #[test]
    fn dynamic_code_in_a_served_shim_fails_closed() {
        let module = Module::new(REGIONS_MODULE, "eval('load another module');");
        let referrer = module_specifier(module.filename()).expect("absolute shim URL");
        let modules = [module];
        let edges =
            module_edges(modules[0].contents(), &referrer, &modules).expect("shim source parses");
        let mut pending = VecDeque::new();
        assert_eq!(
            enqueue_imports(&edges, &mut pending),
            Err("dynamic code construction")
        );
    }

    #[test]
    fn api_barrel_is_retained_when_a_sibling_imports_it() {
        const SIBLING_MODULE: &str = "/rs2b0t/bot/scripts/bot/sibling.js";
        let gated_paths = gated_module_paths();
        let mut modules = shim_modules();
        modules.push(Module::new(SIBLING_MODULE, "import './rs2b0t-api.js';"));
        modules.push(Module::new(BOT_MODULE, "export default class T {}"));
        filter_gated_modules(&mut modules, [BOT_MODULE, SIBLING_MODULE]);
        let retained = gated_paths
            .into_iter()
            .filter(|path| {
                modules
                    .iter()
                    .any(|module| module.filename() == Path::new(path))
            })
            .collect::<Vec<_>>();
        for path in [
            DECLARED_SURFACE_MODULE,
            RS2B0T_API_MODULE,
            "/rs2b0t/bot/api/bank/Bank.js",
            WALK_DESTINATIONS_MODULE,
        ] {
            assert!(
                retained.iter().any(|retained| retained == path),
                "sibling API barrel closure omitted {path}"
            );
        }
    }

    #[test]
    fn main_module_is_an_import_closure_root() {
        let mut modules = shim_modules();
        modules.push(Module::new(BOT_MODULE, "export default class T {}"));
        modules.push(Module::new(
            MAIN_MODULE,
            "import '../../api/map/regions.js';",
        ));
        filter_gated_modules(&mut modules, [MAIN_MODULE]);
        assert!(
            modules
                .iter()
                .any(|module| module.filename() == Path::new(REGIONS_MODULE)),
            "main module imports must retain their served shims"
        );
    }
}
#[cfg(test)]
mod shim_import_graph_memory_tests {
    use super::build_shim_import_graph;

    #[test]
    fn process_graph_retained_allocation_stays_below_36_kib() {
        let mut graph = None;
        let allocation = allocation_counter::measure(|| {
            graph = Some(build_shim_import_graph());
        });
        let graph = graph.expect("graph builder ran");
        assert!(graph.complete, "shim import graph must parse completely");
        assert_eq!(graph.edges.len(), graph.paths.len());

        const RETAINED_LIMIT: i64 = 36 * 1024;
        eprintln!(
            "process shim import graph retained={} bytes; limit={RETAINED_LIMIT}",
            allocation.bytes_current
        );
        assert!(
            allocation.bytes_current <= RETAINED_LIMIT,
            "process shim import graph retains {} bytes, above {RETAINED_LIMIT}",
            allocation.bytes_current
        );
    }
}
