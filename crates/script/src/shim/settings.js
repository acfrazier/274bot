// Settings types + SettingsStore stub for gold script eval (host bag is separate).
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
        throw new Error('not impl: SettingsStore.globalBag');
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
