import { notImpl } from '../shim/_kernel.js';

export const RecoveryHints = {
    takeAnchor() {
        return undefined;
    },
    clear() {
        throw notImpl('RecoveryHints.clear');
    },
};
