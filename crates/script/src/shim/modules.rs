use rustyscript::Module;

/// Canonical path of the user's bot module. Fixed for every card so
/// relative rs2b0t imports (`../../api/...`) resolve to the same shim
/// URLs no matter which script loaded; it is a synthetic specifier only
/// (nothing is read from `/rs2b0t` on this machine).
pub(crate) const BOT_MODULE: &str = "/rs2b0t/bot/scripts/bot/bot.js";
/// Canonical path of the shape wrapper that imports `./bot.js`.
pub(crate) const MAIN_MODULE: &str = "/rs2b0t/bot/scripts/bot/main.js";

/// The prelude eval'd into every isolate before any module loads: the host
/// handle, the bot base classes, `defineBot`, and a recording canvas ctx for
/// `onPaint`. The compat shapes and the shim modules rely on these
/// globals. The classes live here (not in a module) so `extends` and
/// `instanceof` agree with the tick wrapper; `Bot.js` re-exports them.
pub(crate) const PRELUDE: &str = r#"
globalThis.__rs2b0t_host = {};
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
                this._subs = this._subs || Object.create(null);
                const key = String(event);
                (this._subs[key] || (this._subs[key] = [])).push(cb);
            }
    log(message) {
        const h = globalThis.__rs2b0t_host;
        h.log = h.log || [];
        h.log.push(String(message));
    }
    get settings() {
        const bag = globalThis.__rs2b0t_host.settingsBag || {};
        // One view per bag: the accessors read the bag at call time, so the
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
};
globalThis.__rs2b0t_dispatch_native_events = (evs) => {
    const inst = globalThis.__rs_bot;
    const subs = inst && inst._subs;
    if (!subs || !evs) return;
    for (let i = 0; i < evs.length; i++) {
        const e = evs[i];
        const cbs = subs[e.type];
        if (!cbs) continue;
        const payload = e.payload;
        for (let j = 0; j < cbs.length; j++) {
            try {
                const r = cbs[j](payload);
                if (r && typeof r.then === 'function') {
                    r.then(() => {}, (err) => {
                        const h = globalThis.__rs2b0t_host;
                        if (h) h.lastError = String((err && err.message) || err);
                    });
                }
            } catch (_) {}
        }
    }
};
globalThis.TaskBot = class TaskBot extends globalThis.LoopingBot {
    constructor() {
        super();
        this._tasks = [];
    }
    add(...tasks) {
        this._tasks.push(...tasks);
    }
    async loop() {
        for (const task of this._tasks) {
            if (await task.validate()) {
                await task.execute();
                return;
            }
        }
    }
};
globalThis.TreeBot = class TreeBot extends globalThis.LoopingBot {
    root() {
        throw new Error('not impl: TreeBot.root');
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
            h.interact = h.interact || [];
            h.interact.push({
                op: 'key',
                down: ev.type === 'keydown',
                key: String(ev.key || ''),
                code: String(ev.code || ''),
            });
            return true;
        }
        if (ev && (ev.type === 'mousedown' || ev.type === 'mouseup')) {
            const h = globalThis.__rs2b0t_host;
            h.interact = h.interact || [];
            h.interact.push({
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

/// The extra modules that make rs2b0t imports hit our shim, in load order
/// (a module must be registered before anything imports it; the bot's own
/// module is appended by the caller, after these). The paths mirror the
/// real rs2b0t tree under `src/bot/` (scripts live at
/// `src/bot/scripts/<N>/`, so `../../api/...` from a script resolves to
/// `src/bot/api/...`, and the adapter lives at `src/bot/adapter/`).
pub(crate) fn shim_modules() -> Vec<Module> {
    vec![
        // `_kernel.js` imports the Execution park for `runMachine`.
        Module::new(
            "/rs2b0t/bot/api/execution/Execution.js",
            include_str!("execution.js"),
        ),
        Module::new("/rs2b0t/bot/shim/_kernel.js", include_str!("_kernel.js")),
        Module::new("/rs2b0t/bot/geometry/Tile.js", include_str!("tile.js")),
        Module::new("/rs2b0t/bot/api/query/Query.js", include_str!("query.js")),
        Module::new(
            "/rs2b0t/bot/api/execution/EventSignal.js",
            include_str!("event_signal.js"),
        ),
        Module::new(
            "/rs2b0t/bot/adapter/ClientAdapter.js",
            include_str!("client_adapter.js"),
        ),
        Module::new("/rs2b0t/bot/input/Input.js", include_str!("input.js")),
        Module::new("/rs2b0t/bot/api/game/Game.js", include_str!("game.js")),
        Module::new(
            "/rs2b0t/bot/api/inventory/Inventory.js",
            include_str!("inventory.js"),
        ),
        // food before packRules: packRules.shouldEat forwards to shouldEatToUseFood
        Module::new("/rs2b0t/bot/api/combat/food.js", include_str!("food.js")),
        Module::new(
            "/rs2b0t/bot/api/inventory/packRules.js",
            include_str!("pack_rules.js"),
        ),
        Module::new(
            "/rs2b0t/bot/api/acquisition/Tools.js",
            include_str!("tools.js"),
        ),
        Module::new(
            "/rs2b0t/bot/api/skills/Skills.js",
            include_str!("skills.js"),
        ),
        Module::new(
            "/rs2b0t/bot/api/prayer/Prayer.js",
            include_str!("prayer.js"),
        ),
        Module::new("/rs2b0t/bot/api/bank/Bank.js", include_str!("bank.js")),
        Module::new(
            "/rs2b0t/bot/api/bank/bankOps.js",
            include_str!("bank_ops.js"),
        ),
        Module::new(
            "/rs2b0t/bot/api/bank/bankSortRules.js",
            include_str!("bank_sort_rules.js"),
        ),
        Module::new(
            "/rs2b0t/bot/api/bank/Banking.js",
            include_str!("banking.js"),
        ),
        Module::new(
            "/rs2b0t/bot/api/bank/bankRules.js",
            include_str!("bank_rules.js"),
        ),
        Module::new(
            "/rs2b0t/bot/api/bank/bankSort.js",
            include_str!("bank_sort.js"),
        ),
        Module::new(
            "/rs2b0t/bot/api/bank/bankQuestJunk.js",
            include_str!("bank_quest_junk.js"),
        ),
        Module::new("/rs2b0t/bot/api/npcs/Npcs.js", include_str!("npcs.js")),
        Module::new("/rs2b0t/bot/api/model/Npc.js", include_str!("model_npc.js")),
        Module::new("/rs2b0t/bot/api/locs/Locs.js", include_str!("locs.js")),
        Module::new("/rs2b0t/bot/api/model/Loc.js", include_str!("model_loc.js")),
        Module::new(
            "/rs2b0t/bot/api/players/Players.js",
            include_str!("players.js"),
        ),
        Module::new(
            "/rs2b0t/bot/api/model/Player.js",
            include_str!("model_player.js"),
        ),
        Module::new(
            "/rs2b0t/bot/api/grounditems/GroundItems.js",
            include_str!("grounditems.js"),
        ),
        Module::new(
            "/rs2b0t/bot/api/equipment/Equipment.js",
            include_str!("equipment.js"),
        ),
        Module::new("/rs2b0t/bot/api/trade/Trade.js", include_str!("trade.js")),
        Module::new(
            "/rs2b0t/bot/api/trade/drivePartnerTrade.js",
            include_str!("drive_partner_trade.js"),
        ),
        Module::new(
            "/rs2b0t/bot/api/trade/PartnerTrade.js",
            include_str!("partner_trade.js"),
        ),
        Module::new("/rs2b0t/bot/api/shop/Shop.js", include_str!("shop.js")),
        Module::new(
            "/rs2b0t/bot/api/shop/types.js",
            include_str!("shop_types.js"),
        ),
        Module::new(
            "/rs2b0t/bot/api/shop/BuyoutLogic.js",
            include_str!("buyout_logic.js"),
        ),
        Module::new(
            "/rs2b0t/bot/api/ui/dialogue/ChatDialog.js",
            include_str!("chat_dialog.js"),
        ),
        Module::new(
            "/rs2b0t/bot/api/ui/widgets/Modals.js",
            include_str!("modals.js"),
        ),
        Module::new(
            "/rs2b0t/bot/api/ui/questlog/Quests.js",
            include_str!("quests.js"),
        ),
        Module::new(
            "/rs2b0t/bot/api/tasks/ContinueDialog.js",
            include_str!("continue_dialog.js"),
        ),
        Module::new(
            "/rs2b0t/bot/api/tasks/DeathRecovery.js",
            include_str!("death_recovery.js"),
        ),
        Module::new(
            "/rs2b0t/bot/api/tasks/PeriodicBank.js",
            include_str!("periodic_bank.js"),
        ),
        Module::new(
            "/rs2b0t/bot/api/combat/CombatStyle.js",
            include_str!("combat_style.js"),
        ),
        Module::new(
            "/rs2b0t/bot/data/spelldb.js",
            include_str!("data/spelldb.js"),
        ),
        Module::new("/rs2b0t/bot/data/itemdb.js", include_str!("data/itemdb.js")),
        Module::new("/rs2b0t/bot/data/herbs.js", include_str!("data/herbs.js")),
        Module::new("/rs2b0t/bot/data/dropdb.js", include_str!("dropdb.js")),
        Module::new(
            "/rs2b0t/bot/data/cowKillerLocations.js",
            include_str!("cow_killer_locations.js"),
        ),
        Module::new("/rs2b0t/bot/data/shopdb.js", include_str!("shopdb.js")),
        Module::new(
            "/rs2b0t/bot/data/miningRocks.js",
            include_str!("mining_rocks.js"),
        ),
        Module::new(
            "/rs2b0t/bot/data/cookLocations.js",
            include_str!("cook_locations_data.js"),
        ),
        Module::new(
            "/rs2b0t/bot/data/runeCraftLocations.js",
            include_str!("rune_craft_locations.js"),
        ),
        Module::new(
            "/rs2b0t/bot/data/pickpocketTargets.js",
            include_str!("data/pickpocket_targets.js"),
        ),
        Module::new(
            "/rs2b0t/bot/data/woodcuttingLocations.js",
            include_str!("data/woodcutting_locations.js"),
        ),
        Module::new(
            "/rs2b0t/bot/api/combat/CombatStyleLogic.js",
            include_str!("combat_style_logic.js"),
        ),
        // food.js registered earlier (before packRules)
        Module::new(
            "/rs2b0t/bot/api/combat/Special.js",
            include_str!("special.js"),
        ),
        Module::new(
            "/rs2b0t/bot/api/combat/rangedSettings.js",
            include_str!("ranged_settings.js"),
        ),
        Module::new(
            "/rs2b0t/bot/api/combat/fightUpkeep.js",
            include_str!("fight_upkeep.js"),
        ),
        Module::new(
            "/rs2b0t/bot/api/combat/eatTiming.js",
            include_str!("eat_timing.js"),
        ),
        Module::new(
            "/rs2b0t/bot/api/combat/boostPotions.js",
            include_str!("boost_potions.js"),
        ),
        Module::new(
            "/rs2b0t/bot/api/combat/keepList.js",
            include_str!("keep_list.js"),
        ),
        Module::new(
            "/rs2b0t/bot/api/combat/equipment.js",
            include_str!("combat_equipment.js"),
        ),
        Module::new(
            "/rs2b0t/bot/api/combat/meleeWeapons.js",
            include_str!("melee_weapons.js"),
        ),
        Module::new(
            "/rs2b0t/bot/api/combat/ranged.js",
            include_str!("ranged.js"),
        ),
        Module::new(
            "/rs2b0t/bot/api/sustain/Sustain.js",
            include_str!("sustain.js"),
        ),
        Module::new(
            "/rs2b0t/bot/api/combat/hunting/combat.js",
            include_str!("hunting_combat.js"),
        ),
        Module::new(
            "/rs2b0t/bot/api/combat/hunting/sites.js",
            include_str!("hunting_sites.js"),
        ),
        Module::new(
            "/rs2b0t/bot/api/combat/hunting/logic.js",
            include_str!("hunting_logic.js"),
        ),
        Module::new(
            "/rs2b0t/bot/api/combat/hunting/guarded.js",
            include_str!("hunting_guarded.js"),
        ),
        Module::new(
            "/rs2b0t/bot/api/combat/hunting/supply.js",
            include_str!("hunting_supply.js"),
        ),
        Module::new(
            "/rs2b0t/bot/api/firemaking/LightFire.js",
            include_str!("light_fire.js"),
        ),
        Module::new(
            "/rs2b0t/bot/api/firemaking/Firemaking.js",
            include_str!("firemaking.js"),
        ),
        Module::new(
            "/rs2b0t/bot/api/magic/Autocast.js",
            include_str!("autocast.js"),
        ),
        Module::new(
            "/rs2b0t/bot/api/chatbox/gameMessages.js",
            include_str!("game_messages.js"),
        ),
        Module::new(
            "/rs2b0t/bot/runtime/RecoveryHints.js",
            include_str!("recovery_hints.js"),
        ),
        Module::new(
            "/rs2b0t/bot/runtime/Supervisor.js",
            include_str!("supervisor.js"),
        ),
        Module::new(
            "/rs2b0t/bot/api/bank/BankLocations.js",
            include_str!("bank_locations.js"),
        ),
        Module::new(
            "/rs2b0t/bot/api/cooking/CookLocations.js",
            include_str!("cook_locations.js"),
        ),
        Module::new(
            "/rs2b0t/bot/api/thieving/targets.js",
            include_str!("thieving_targets.js"),
        ),
        Module::new(
            "/rs2b0t/bot/api/thieving/stealRules.js",
            include_str!("steal_rules.js"),
        ),
        Module::new(
            "/rs2b0t/bot/api/walking/Traversal.js",
            include_str!("traversal.js"),
        ),
        Module::new(
            "/rs2b0t/bot/api/thieving/cakeStallData.js",
            include_str!("cake_stall_data.js"),
        ),
        Module::new(
            "/rs2b0t/bot/api/thieving/CakeStall.js",
            include_str!("cake_stall.js"),
        ),
        Module::new(
            "/rs2b0t/bot/api/loadout/loadoutSetting.js",
            include_str!("loadout_setting.js"),
        ),
        Module::new(
            "/rs2b0t/bot/api/loadout/loadoutPlan.js",
            include_str!("loadout_plan.js"),
        ),
        Module::new(
            "/rs2b0t/bot/paint/levelProgress.js",
            include_str!("level_progress.js"),
        ),
        Module::new(
            "/rs2b0t/bot/api/market/catalog.js",
            include_str!("catalog.js"),
        ),
        Module::new(
            "/rs2b0t/bot/api/market/MarketMaker.js",
            include_str!("market_maker.js"),
        ),
        Module::new(
            "/rs2b0t/bot/runtime/RunManager.js",
            include_str!("run_manager.js"),
        ),
        Module::new(
            "/rs2b0t/bot/runtime/Settings.js",
            include_str!("settings.js"),
        ),
        Module::new(
            "/rs2b0t/bot/api/ai/quests/engine/QuestEngine.js",
            include_str!("quest_engine.js"),
        ),
        Module::new(
            "/rs2b0t/bot/api/duel/Duel.js",
            include_str!("duel_unimplemented.js"),
        ),
        Module::new(
            "/rs2b0t/bot/api/duel/ClueDuel.js",
            include_str!("clue_duel.js"),
        ),
        Module::new(
            "/rs2b0t/bot/api/ai/clues/bankAccess.js",
            include_str!("clue_bank_access.js"),
        ),
        Module::new(
            "/rs2b0t/bot/api/ai/clues/duelTravel.js",
            include_str!("clue_duel_travel.js"),
        ),
        Module::new(
            "/rs2b0t/bot/api/ai/clues/SolveClue.js",
            include_str!("solve_clue.js"),
        ),
        Module::new(
            "/rs2b0t/bot/api/ai/clues/cluePaint.js",
            include_str!("clue_paint.js"),
        ),
        Module::new(
            "/rs2b0t/bot/api/ai/clues/data/cluedb.js",
            include_str!("cluedb.js"),
        ),
        Module::new(
            "/rs2b0t/bot/api/ai/quests/exec/primitives.js",
            include_str!("primitives.js"),
        ),
        Module::new(
            "/rs2b0t/bot/api/walking/DirectNavigator.js",
            include_str!("direct_navigator.js"),
        ),
        Module::new(
            "/rs2b0t/bot/event/webwalk/DirectNavigator.js",
            include_str!("direct_navigator.js"),
        ),
        Module::new("/rs2b0t/bot/api/walking/Reach.js", include_str!("reach.js")),
        Module::new(
            "/rs2b0t/bot/event/webwalk/geometry/Reachability.js",
            include_str!("reachability.js"),
        ),
        Module::new(
            "/rs2b0t/bot/event/webwalk/walkOpening.js",
            include_str!("walk_opening.js"),
        ),
        Module::new(
            "/rs2b0t/bot/event/webwalk/Navigator.js",
            include_str!("navigator.js"),
        ),
        Module::new("/rs2b0t/bot/api/tasks/Anchor.js", include_str!("anchor.js")),
        Module::new("/rs2b0t/bot/api/bot/Bot.js", include_str!("bot.js")),
        Module::new("/rs2b0t/bot/paint/Paint.js", include_str!("paint.js")),
        Module::new("/rs2b0t/bot/paint/jive.js", include_str!("jive.js")),
        Module::new(
            "/rs2b0t/bot/paint/paintLogic.js",
            include_str!("paintLogic.js"),
        ),
        Module::new(
            "/rs2b0t/bot/runtime/ScriptRunner.js",
            include_str!("script_runner.js"),
        ),
        Module::new(
            "/rs2b0t/bot/runtime/BotHost.js",
            include_str!("bot_host.js"),
        ),
        Module::new(
            "/rs2b0t/bot/scripts/bot/declared_surface.js",
            include_str!("declared_surface.js"),
        ),
        Module::new(
            "/rs2b0t/bot/scripts/bot/rs2b0t-api.js",
            include_str!("rs2b0t_api.js"),
        ),
    ]
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
pub(crate) fn remap_catalog_imports(source: &str) -> String {
    remap_hash_bot(&remap_rs2b0t_api(source))
}
