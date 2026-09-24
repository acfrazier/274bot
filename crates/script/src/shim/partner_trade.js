// Name-map `PartnerTrade`: partner predicates, settings parsers and the
// first-screen decisions live in `script::partner_trade`. Trade HUD actions
// stay on `Trade` / `drivePartnerTrade`.
import { notImpl } from '../../shim/_kernel.js';

const pt = (name, ...args) => globalThis.__rs2b0t_partner_trade(name, ...args);

export const DEFAULT_TRADE_RANGE = pt('DEFAULT_TRADE_RANGE');
export const MULE_MODE_OPTIONS = Object.freeze(pt('MULE_MODE_OPTIONS'));

export const parsePartnerList = (raw) => pt('parsePartnerList', raw);
export const namesMatch = (a, b) => pt('namesMatch', a, b);
export const isConfiguredPartner = (name, partners) => pt('isConfiguredPartner', name, partners);
export const countOfferByName = (items, itemName) => pt('countOfferByName', items, itemName);
export const decideReceiverOfferScreen = (opts) => pt('decideReceiverOfferScreen', opts);
export const decideGiverOfferScreen = (myOfferSlots) => pt('decideGiverOfferScreen', myOfferSlots);
export const parseMuleMode = (raw) => pt('parseMuleMode', raw);
export const muleGathererHandoffActive = (mode, partners, powerMode) =>
    pt('muleGathererHandoffActive', mode, partners, powerMode);
export const muleReceiverActive = (mode, partners) => pt('muleReceiverActive', mode, partners);
export const muleCookerActive = (mode, partners) => pt('muleCookerActive', mode, partners);
export const muleSupplierActive = (mode, partners, powerMode) =>
    pt('muleSupplierActive', mode, partners, powerMode);
export const muleNonGathererActive = (mode, partners) => pt('muleNonGathererActive', mode, partners);

export function countOfferMatching() {
    throw notImpl('PartnerTrade.countOfferMatching', 'caller match predicate per offer slot');
}
