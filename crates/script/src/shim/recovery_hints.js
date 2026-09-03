import { notV1 } from '../shim/_kernel.js';

export const RecoveryHints = {
    takeAnchor() {
        throw notV1('RecoveryHints.takeAnchor');
    },
    clear() {
        throw notV1('RecoveryHints.clear');
    },
};
