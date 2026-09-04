import { notImpl } from '../../shim/_kernel.js';
import { Trade } from './Trade.js';

export async function driveActivePartnerTrade() {
    if (!Trade.active()) return;
    throw notImpl('driveActivePartnerTrade');
}
