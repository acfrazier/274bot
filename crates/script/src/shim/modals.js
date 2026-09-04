import { snap, queue, proxy } from '../../../shim/_kernel.js';

export const Modals = proxy('Modals', {
    main() {
        const id = snap().main_modal_id;
        return typeof id === 'number' ? id : -1;
    },
    isOpen() {
        return Modals.main() !== -1;
    },
    close() {
        queue({ op: 'close-modal' });
        return true;
    },
});
