// ScriptRunner: `stop()` flags the host handle (the isolate thread logs
// the clear hook; Stop dispatch lands with the Execution wiring);
// `paintControls` appends Pause/Stop hint rows to a paint frame.
const host = () => globalThis.__rs2b0t_host || {};

export const ScriptRunner = {
    stop(reason) {
        host().stopRequested = true;
        if (typeof reason === 'string') {
            host().stopReason = reason;
        }
    },
    paintControls(p) {
        p.gap();
        p.row('Pause/Resume: host · Stop: ScriptRunner.stop()');
    },
};
