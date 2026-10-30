// ScriptRunner: `stop()` flags the host handle (the isolate thread logs
// the clear hook; Stop dispatch lands with the Execution wiring);
// `paintControls` advertises the host's Pause/Resume and Stop controls as
// paint buttons with fixed ids. The host owns their labels and commands;
// a click on them never returns to the script.
import { HOST_CONTROLS, snap } from '../shim/_kernel.js';

const host = () => globalThis.__rs2b0t_host || {};
const runtime = (...args) => globalThis.__rs2b0t_script_runtime(...args);
let context;
let contextGeneration;

function contextView() {
    const generation = runtime('generation');
    if (generation === null) {
        context = undefined;
        contextGeneration = undefined;
        return null;
    }
    if (context && contextGeneration === generation) return context;
    contextGeneration = generation;
    context = Object.defineProperties({}, {
        state: { enumerable: true, get: () => runtime('state', generation) },
        aborted: { enumerable: true, get: () => runtime('aborted', generation) },
        stopReason: { enumerable: true, get: () => runtime('stopReason', generation) },
        activeEvent: {
            enumerable: true,
            get: () => runtime('isCurrent', generation)
                ? (snap().guardian_active_event ?? null)
                : null,
        },
        watchdogHold: {
            enumerable: true,
            get: () => runtime('watchdogHold', generation),
            set: (value) => runtime('setWatchdogHold', generation, value),
        },
        abortWaiters: {
            enumerable: true,
            value: () => runtime('abortWaiters', generation),
        },
    });
    return context;
}

globalThis.__rs2b0t_script_context_view = contextView;

export const ScriptRunner = {
    get state() {
        return runtime('state');
    },
    get meta() {
        return runtime('meta');
    },
    get ctx() {
        return contextView();
    },
    pause() {
        runtime('pause');
    },
    resume() {
        runtime('resume');
    },
    stop(reason) {
        host().stopRequested = true;
        if (typeof reason === 'string') {
            host().stopReason = reason;
        }
    },
    paintControls(p) {
        p.buttons(HOST_CONTROLS);
    },
};
