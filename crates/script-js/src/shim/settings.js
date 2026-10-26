// Settings types + SettingsStore stub for gold script eval (host bag is separate).
import { notImpl, notImplValue } from '../shim/_kernel.js';
import { tileFromPosted } from '../geometry/Tile.js';

export class SettingsBag {
    constructor(values = {}) {
        this.values = values;
    }

    bool(key, fallback = false) {
        const v = this.values[key];
        return typeof v === 'boolean' ? v : fallback;
    }

    num(key, fallback = 0) {
        const v = this.values[key];
        return typeof v === 'number' && Number.isFinite(v) ? v : fallback;
    }

    str(key, fallback = '') {
        const v = this.values[key];
        return typeof v === 'string' ? v : fallback;
    }

    list(key, fallback = []) {
        const v = this.values[key];
        return Array.isArray(v) ? v : fallback;
    }

    tile(key, fallback = null) {
        return tileFromPosted(this.values[key]) ?? fallback;
    }
}

class SettingsStoreImpl {
    resolve(_name, schema) {
        const bag = (globalThis.__rs2b0t_host || {}).settingsBag || {};
        const out = {};
        for (const key of Object.keys(schema || {})) {
            out[key] = Object.prototype.hasOwnProperty.call(bag, key) ? bag[key] : null;
        }
        return out;
    }

    displayString(_name, key, def) {
        const bag = (globalThis.__rs2b0t_host || {}).settingsBag || {};
        if (Object.prototype.hasOwnProperty.call(bag, key)) return settingToString(bag[key]);
        return settingToString(def?.default ?? '');
    }

    saved(_name, key) {
        const bag = (globalThis.__rs2b0t_host || {}).settingsBag || {};
        return Object.prototype.hasOwnProperty.call(bag, key) ? settingToString(bag[key]) : undefined;
    }

    globalBag() {
        return new SettingsBag((globalThis.__rs2b0t_host || {}).globalSettingsBag || {});
    }
}

function settingToString(value) {
    if (typeof value === 'boolean') return value ? 'true' : 'false';
    if (Array.isArray(value)) return value.join(', ');
    if (
        value &&
        typeof value === 'object' &&
        Object.prototype.hasOwnProperty.call(value, 'x') &&
        Object.prototype.hasOwnProperty.call(value, 'z') &&
        Object.prototype.hasOwnProperty.call(value, 'level')
    ) {
        return `${value.x},${value.z},${value.level}`;
    }
    return String(value);
}

export const SettingsStore = new SettingsStoreImpl();
export function settingOptionLabel(def, value) {
    throw notImpl('settingOptionLabel');
}

export const GLOBAL_SETTINGS_CORE = notImplValue('GLOBAL_SETTINGS_CORE');
export const NAV_SETTINGS = notImplValue('NAV_SETTINGS');
export const GLOBAL_SETTINGS = notImplValue('GLOBAL_SETTINGS');
export const MAP_PICKER_SETTINGS_NS = notImplValue('MAP_PICKER_SETTINGS_NS');
export const MAP_PICKER_SETTINGS = notImplValue('MAP_PICKER_SETTINGS');
export const LOADOUT_SETTINGS_NS = notImplValue('LOADOUT_SETTINGS_NS');
export const LOADOUT_SETTINGS = notImplValue('LOADOUT_SETTINGS');
export const PRICE_BOOK_SETTINGS_NS = notImplValue('PRICE_BOOK_SETTINGS_NS');
export const PRICE_BOOK_SETTINGS = notImplValue('PRICE_BOOK_SETTINGS');

