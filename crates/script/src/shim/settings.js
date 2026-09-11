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

    globalBag() {
        throw new Error('not impl: SettingsStore.globalBag');
    }
}

export const SettingsStore = new SettingsStoreImpl();
