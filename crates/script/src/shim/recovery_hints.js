import { notV1 } from '../shim/_kernel.js';

export const RecoveryHints = {
    takeAnchor() {
        return undefined;
    },
    clear() {
        throw notV1('RecoveryHints.clear');
    },
};
