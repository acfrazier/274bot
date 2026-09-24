// Our Modals module. close / closeIfOpen are one Rust step machine each:
// the shim coerces the kind, awaits the settlement and reports bool vs void.
// Rust queues the one close-modal, watches the captured root and owns the
// window.
import { notImpl, proxy, snap, runMachine } from '../../../shim/_kernel.js';

function settle(kind, out) {
    if (kind === 'closeIfOpen') return undefined;
    return out.kind === 'done' && out.value.result === true;
}

async function run(kind) {
    const out = await runMachine('modals', { kind });
    if (out.kind === 'refused') throw notImpl('Modals.' + kind, out.reason);
    return settle(kind, out);
}

export const Modals = proxy('Modals', {
    main() {
        const id = snap().main_modal_id;
        return typeof id === 'number' ? id : -1;
    },
    isOpen() {
        return Modals.main() !== -1;
    },
    close() {
        return run('close');
    },
    closeIfOpen() {
        return run('closeIfOpen');
    },
});
