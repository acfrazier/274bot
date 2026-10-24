// The native nearest-world bank machine selects the bank access kind, walks,
// opens it and waits for the fresh bank page. This adapter only preserves the
// catalog callback signature.
import { Bank } from '../../bank/Bank.js';

export function openClueBank(_log) {
    return Bank.openNearestWorld();
}
