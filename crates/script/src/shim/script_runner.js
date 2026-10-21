// ScriptRunner: `stop()` flags the host handle (the isolate thread logs
// the clear hook; Stop dispatch lands with the Execution wiring);
// `paintControls` advertises the host's Pause/Resume and Stop controls as
// paint buttons with fixed ids. The host owns their labels and commands;
// a click on them never returns to the script.
import { HOST_CONTROLS, notImpl } from '../shim/_kernel.js';

const host = () => globalThis.__rs2b0t_host || {};

export const ScriptRunner = {
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
export function loopReadyOrDetached() {
    throw notImpl('loopReadyOrDetached');
}

export function stopReasonOf(reason) {
    throw notImpl('stopReasonOf');
}
