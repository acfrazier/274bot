//! rs2b0t import-remap shim: extra rustyscript modules that stand in for
//! the rs2b0t api tree. Their names (`../../api/...`, `../../paint/...`,
//! `../../runtime/...`) and the `@rs2b0t/api` bundle resolve to these
//! modules; the rs2b0t sources are never executed. Missing members throw
//! `not v1: <name>` — never a fake value.

use rustyscript::Module;

/// Canonical path of the user's bot module. Fixed for every card so
/// relative rs2b0t imports (`../../api/...`) resolve to the same shim
/// URLs no matter which script loaded; it is a synthetic specifier only
/// (nothing is read from `/rs2b0t` on this machine).
pub(crate) const BOT_MODULE: &str = "/rs2b0t/bot/scripts/bot/bot.js";
/// Canonical path of the shape wrapper that imports `./bot.js`.
pub(crate) const MAIN_MODULE: &str = "/rs2b0t/bot/scripts/bot/main.js";

/// The prelude eval'd into every isolate before any module loads: the host
/// handle, the bot base classes, `defineBot`, and a no-op canvas ctx for
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
    onStart() {}
    loop() {}
    log(message) {
        const h = globalThis.__rs2b0t_host;
        h.log = h.log || [];
        h.log.push(String(message));
    }
    get settings() {
        // The shim serves the script's own defaults; per-run settings
        // panes are not v1.
        return {
            str(name, fallback = '') {
                return fallback;
            },
        };
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
    loop() {
        for (const task of this._tasks) {
            if (task.validate()) {
                task.execute();
                return;
            }
        }
    }
};
globalThis.TreeBot = class TreeBot extends globalThis.LoopingBot {};
globalThis.__dummy_ctx = {
    fillRect() {},
    fillText() {},
    measureText() { return { width: 7 }; },
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
        Module::new("/rs2b0t/bot/api/game/Game.js", include_str!("game.js")),
        Module::new(
            "/rs2b0t/bot/api/inventory/Inventory.js",
            include_str!("inventory.js"),
        ),
        Module::new(
            "/rs2b0t/bot/api/skills/Skills.js",
            include_str!("skills.js"),
        ),
        Module::new(
            "/rs2b0t/bot/api/execution/Execution.js",
            include_str!("execution.js"),
        ),
        Module::new("/rs2b0t/bot/api/bank/Bank.js", include_str!("bank.js")),
        Module::new(
            "/rs2b0t/bot/api/bank/Banking.js",
            include_str!("banking.js"),
        ),
        Module::new(
            "/rs2b0t/bot/api/execution/EventSignal.js",
            include_str!("event_signal.js"),
        ),
        Module::new(
            "/rs2b0t/bot/adapter/ClientAdapter.js",
            include_str!("client_adapter.js"),
        ),
        Module::new("/rs2b0t/bot/api/bot/Bot.js", include_str!("bot.js")),
        Module::new("/rs2b0t/bot/paint/Paint.js", include_str!("paint.js")),
        Module::new(
            "/rs2b0t/bot/paint/paintLogic.js",
            include_str!("paintLogic.js"),
        ),
        Module::new(
            "/rs2b0t/bot/runtime/ScriptRunner.js",
            include_str!("script_runner.js"),
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

/// One recorded paint frame (`Paint.begin(...)` ... `end()`): the title,
/// the accent colour, and the rows (gap rows are empty lines). No canvas —
/// the host reads it off `__rs2b0t_host.paint` for the script paint views.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct ScriptPaint {
    pub title: Option<String>,
    pub accent: Option<String>,
    pub lines: Vec<String>,
}

/// One interact request the shim `Bank`/`Banking` modules queue on the
/// host handle (`__rs2b0t_host.interact`); the isolate thread forwards the
/// queue to the host after each tick, and host-play dispatches each op
/// through the slot Driver. Missing targets fail closed at dispatch (no
/// matching loc/npc/item row → nothing is sent).
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(tag = "op")]
pub enum InteractReq {
    /// Open the bank booth loc at `(x, z, level)` with its Use-quickly op.
    #[serde(rename = "open-booth")]
    OpenBooth { x: i32, z: i32, level: i32 },
    /// Use a packed stand the player is adjacent to: a booth loc
    /// (Use-quickly) or a teller NPC (its 1-based op slot from the pack;
    /// `choose` is the dialog option the op's dialogue needs, deferred).
    #[serde(rename = "open-stand")]
    OpenStand {
        x: i32,
        z: i32,
        level: i32,
        kind: String,
        name: Option<String>,
        /// The stand's 1-based access op slot (booth Use-quickly or the
        /// teller NPC op from the pack).
        stand_op: Option<i32>,
        choose: Option<String>,
    },
    /// Walk to the packed stand tile through the slot's traveller with
    /// default options (no teleports, no wilderness, no bank fetch).
    #[serde(rename = "walk")]
    Walk { x: i32, z: i32, level: i32 },
    /// Deposit-all the bank-side item named `name`.
    #[serde(rename = "deposit")]
    Deposit { name: String },
    /// Withdraw the bank item named `name` with the action label
    /// (`Withdraw All` / `Withdraw 10` / `Withdraw 1`).
    #[serde(rename = "withdraw")]
    Withdraw { name: String, action: String },
    /// Interact with the held item named `name` using the action label
    /// (`Bury`, `Wear`, …). The host resolves the name through ObjNames
    /// and dispatches the item's menu op (rs2b0t `Item.interact`).
    #[serde(rename = "held")]
    Held { name: String, action: String },
    /// Close the open bank modal.
    #[serde(rename = "close")]
    Close,
}
