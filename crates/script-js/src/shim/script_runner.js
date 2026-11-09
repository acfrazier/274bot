// ScriptRunner: `stop()` flags the host handle (the isolate thread logs
// the clear hook; Stop dispatch lands with the Execution wiring);
// `paintControls` advertises the host's Pause/Resume and Stop controls as
// paint buttons with fixed ids. The host owns their labels and commands;
// a click on them never returns to the script. The run context view
// (`ctx`) is built, cached per generation and read by the native runtime
// dispatcher; this shim keeps no state.
import { HOST_CONTROLS } from '../shim/_kernel.js';

const host = () => globalThis.__rs2b0t_host || {};
const runtime = (operation) => globalThis.__rs2b0t_script_runtime(operation);

export const ScriptRunner = {
    get state() {
        return runtime('state');
    },
    get meta() {
        return runtime('meta');
    },
    get ctx() {
        return runtime('contextView');
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
