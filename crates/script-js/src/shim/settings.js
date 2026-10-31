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
    resolve(name, schema) {
        const dispatch = globalThis.__rs2b0t_settings_store;
        if (typeof dispatch !== 'function') {
            throw new Error('SettingsStore native service is unavailable');
        }
        return dispatch('resolve', name, schema);
    }

    displayString(_name, key, def) {
        const bag = (globalThis.__rs2b0t_host || {}).settingsBag || {};
        if (Object.prototype.hasOwnProperty.call(bag, key)) return settingToString(bag[key]);
        return settingToString(def?.default ?? '');
    }

    save(name, key, rawString) {
        const dispatch = globalThis.__rs2b0t_settings_store;
        if (typeof dispatch !== 'function') {
            throw new Error('SettingsStore native service is unavailable');
        }
        dispatch('save', name, key, rawString);
    }

    onChange(listener) {
        const dispatch = globalThis.__rs2b0t_settings_store;
        if (typeof dispatch !== 'function') {
            throw new Error('SettingsStore native service is unavailable');
        }
        dispatch('subscribe', listener);
        return () => dispatch('unsubscribe', listener);
    }

    saved(name, key) {
        const dispatch = globalThis.__rs2b0t_settings_store;
        if (typeof dispatch === 'function') {
            const raw = dispatch('saved', name, key);
            return raw === null ? undefined : raw;
        }
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

