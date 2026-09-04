//! rs2b0t import-remap shim: extra rustyscript modules that stand in for
//! the rs2b0t api tree. Their names (`../../api/...`, `../../paint/...`,
//! `../../runtime/...`) and the `@rs2b0t/api` bundle resolve to these
//! modules; the rs2b0t sources are never executed. Missing members throw
//! `not impl: <throw reason>` — never a fake value.

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
        return {
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
                if (v && typeof v === 'object' && typeof v.x === 'number') return v;
                return fallback;
            },
            list(name, fallback = []) {
                const v = bag[name];
                return Array.isArray(v) ? v : fallback;
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
    async loop() {
        for (const task of this._tasks) {
            if (task.validate()) {
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
        Module::new("/rs2b0t/bot/shim/_kernel.js", include_str!("_kernel.js")),
        Module::new("/rs2b0t/bot/geometry/Tile.js", include_str!("tile.js")),
        Module::new("/rs2b0t/bot/api/query/Query.js", include_str!("query.js")),
        Module::new(
            "/rs2b0t/bot/api/execution/Execution.js",
            include_str!("execution.js"),
        ),
        Module::new(
            "/rs2b0t/bot/api/execution/EventSignal.js",
            include_str!("event_signal.js"),
        ),
        Module::new(
            "/rs2b0t/bot/adapter/ClientAdapter.js",
            include_str!("client_adapter.js"),
        ),
        Module::new("/rs2b0t/bot/api/game/Game.js", include_str!("game.js")),
        Module::new(
            "/rs2b0t/bot/api/inventory/Inventory.js",
            include_str!("inventory.js"),
        ),
        Module::new(
            "/rs2b0t/bot/api/skills/Skills.js",
            include_str!("skills.js"),
        ),
        Module::new("/rs2b0t/bot/api/bank/Bank.js", include_str!("bank.js")),
        Module::new(
            "/rs2b0t/bot/api/bank/Banking.js",
            include_str!("banking.js"),
        ),
        Module::new("/rs2b0t/bot/api/npcs/Npcs.js", include_str!("npcs.js")),
        Module::new("/rs2b0t/bot/api/locs/Locs.js", include_str!("locs.js")),
        Module::new(
            "/rs2b0t/bot/api/players/Players.js",
            include_str!("players.js"),
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
        Module::new("/rs2b0t/bot/api/shop/Shop.js", include_str!("shop.js")),
        Module::new(
            "/rs2b0t/bot/api/ui/dialogue/ChatDialog.js",
            include_str!("chat_dialog.js"),
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
        Module::new(
            "/rs2b0t/bot/data/pickpocketTargets.js",
            include_str!("data/pickpocket_targets.js"),
        ),
        Module::new(
            "/rs2b0t/bot/api/combat/CombatStyleLogic.js",
            include_str!("combat_style_logic.js"),
        ),
        Module::new("/rs2b0t/bot/api/combat/food.js", include_str!("food.js")),
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
            "/rs2b0t/bot/api/bank/BankLocations.js",
            include_str!("bank_locations.js"),
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
            "/rs2b0t/bot/runtime/Settings.js",
            include_str!("settings.js"),
        ),
        Module::new(
            "/rs2b0t/bot/api/ai/quests/engine/QuestEngine.js",
            include_str!("quest_engine.js"),
        ),
        Module::new(
            "/rs2b0t/bot/api/ai/clues/SolveClue.js",
            include_str!("solve_clue.js"),
        ),
        Module::new(
            "/rs2b0t/bot/api/walking/Traversal.js",
            include_str!("traversal.js"),
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
        Module::new("/rs2b0t/bot/api/tasks/Anchor.js", include_str!("anchor.js")),
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
    /// Open the nearest Use-quickly loc on the player's plane. Tile
    /// fields are unused (host finds the loc); JS may omit them.
    #[serde(rename = "open-booth")]
    OpenBooth {
        #[serde(default)]
        x: i32,
        #[serde(default)]
        z: i32,
        #[serde(default)]
        level: i32,
    },
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
    /// Packed nav (`Traveller` / `ScriptWalkArm`). `allow_teleports` is
    /// the only FindOptions opt-in the catalog forwards (default off).
    #[serde(rename = "walk")]
    Walk {
        x: i32,
        z: i32,
        level: i32,
        #[serde(default)]
        allow_teleports: bool,
    },
    /// Scene `try_move` packet (`Interactions::walk`). Catalog
    /// `Traversal.walkTo` — not Traveller.
    #[serde(rename = "walk-to")]
    WalkTo { x: i32, z: i32, level: i32 },
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
    /// Interact with an NPC by name using an action label (`Pick`, …).
    #[serde(rename = "npc")]
    Npc {
        name: String,
        action: String,
        index: Option<i32>,
    },
    /// Interact with a loc at `(x, z, level)` using an action label.
    #[serde(rename = "loc")]
    Loc {
        x: i32,
        z: i32,
        level: i32,
        action: String,
    },
    /// Interact with a ground item at `(x, z, level)` using an action label.
    #[serde(rename = "obj")]
    Obj {
        x: i32,
        z: i32,
        level: i32,
        name: Option<String>,
        action: String,
    },
    /// Interact with a player by name using an action label.
    #[serde(rename = "player")]
    Player { name: String, action: String },
    /// Use a held inventory item on a scene target (`Game.castOnItem`).
    #[serde(rename = "use-on")]
    UseOn {
        name: String,
        kind: String,
        target_name: Option<String>,
        x: i32,
        z: i32,
        level: i32,
        index: Option<i32>,
    },
    /// Use a widget (spell / interface button) on a scene target.
    #[serde(rename = "use-widget-on")]
    UseWidgetOn {
        component_id: i32,
        kind: String,
        target_name: Option<String>,
        x: i32,
        z: i32,
        level: i32,
        index: Option<i32>,
    },
    /// Continue the open chat dialog.
    #[serde(rename = "continue")]
    ContinueDialog,
    /// Answer the chat modal's `option`-th choice (1-based).
    #[serde(rename = "answer")]
    Answer { option: i32 },
    /// Press an interface button by component id.
    #[serde(rename = "if-button")]
    IfButton { component_id: i32 },
    /// Close the open main/side/chat modal (not the bank).
    #[serde(rename = "close-modal")]
    CloseModal,
    /// Answer the open count dialog with `value`.
    #[serde(rename = "answer-count")]
    AnswerCount { value: i32 },
    /// Switch the active side tab.
    #[serde(rename = "side-tab")]
    SideTab { tab: i32 },
    /// Wear/wield an inventory item by resolved name.
    #[serde(rename = "wear")]
    Wear { name: String },
    /// Toggle run on/off.
    #[serde(rename = "set-run")]
    SetRun { on: bool },
    /// Toggle auto-retaliate on/off.
    #[serde(rename = "set-retaliate")]
    SetRetaliate { on: bool },
    /// Toggle bank withdraw-as-note on/off.
    #[serde(rename = "set-note-mode")]
    SetNoteMode { on: bool },
    /// Host-side orbit yaw write (`client.orbit_camera_yaw`); no opcode.
    #[serde(rename = "set-camera-yaw")]
    SetCameraYaw { yaw: i32 },
}
