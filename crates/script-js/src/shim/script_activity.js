// ScriptActivity is a thin run-fenced handle; the slot context owns its child and outcome.
import { LoopingBot, TaskBot } from '../api/bot/Bot.js';
import { EventSignal } from '../api/execution/EventSignal.js';
import { ScriptAborted } from '../api/execution/Execution.js';
import { Sustain } from '../api/sustain/Sustain.js';
import { BotHost } from './BotHost.js';
import { RunManager } from './RunManager.js';
import { Scheduler } from './Scheduler.js';
import { SettingsBag } from './Settings.js';

const native = (...args) => globalThis.__rs2b0t_script_activity(...args);
class ActivityFinished extends Error {}

export class ScriptActivity {
    #generation;
    #log;

    constructor(log) {
        this.#generation = native('construct');
        this.#log = log;
    }

    get child() {
        return native('child', this, this.#generation);
    }

    get outcome() {
        return native('outcome', this, this.#generation);
    }

    get currentTask() {
        return native('currentTask', this, this.#generation, TaskBot);
    }

    async start(meta, overrides) {
        await native(
            'start', this, this.#generation, meta, overrides, this.#log,
            LoopingBot, SettingsBag, Sustain, EventSignal, RunManager, Scheduler,
            ScriptAborted, BotHost, ActivityFinished,
        );
    }

    async loop() {
        await native(
            'loop', this, this.#generation, Sustain, EventSignal, RunManager,
            Scheduler, ScriptAborted, BotHost, ActivityFinished,
        );
    }

    stop() {
        native('stop', this, this.#generation, Sustain, EventSignal, RunManager);
    }

    interrupt(reason) {
        native('interrupt', this, this.#generation, reason);
    }
}
