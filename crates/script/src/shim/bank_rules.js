export {
    parseBankStrategy,
    PERIODIC_BANK_SETTINGS,
    COMMON_BANK_LOOT,
    matchesCommonBankLoot,
    depositMatcher,
    depositAllExcept,
} from './Banking.js';
import { notImpl } from '../../shim/_kernel.js';

export function shouldBankNow() {
    throw notImpl('bankRules.shouldBankNow');
}

export function isDisposableGatherJunk() {
    throw notImpl('bankRules.isDisposableGatherJunk');
}
