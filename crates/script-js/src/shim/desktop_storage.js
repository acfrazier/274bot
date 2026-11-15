// rs2b0t's desktop shell shares one storage between its windows. 274bot keeps what a script
// saves here in the SettingsStore, so saved script state survives a restart. One saved value is
// at most 64 KiB (the store refuses a longer one). Only the two members catalog scripts call
// are bound: getItem and setItem. The rest refuse.
import { notImpl } from '../shim/_kernel.js';
import { SettingsStore } from './Settings.js';

const NAMESPACE = 'desktopStorage';

function storage() {
    return {
        getItem: (key) => SettingsStore.saved(NAMESPACE, key) ?? null,
        setItem: (key, value) => SettingsStore.save(NAMESPACE, key, value),
        removeItem() {
            throw notImpl('desktopStorage.removeItem');
        },
        keys() {
            throw notImpl('desktopStorage.keys');
        },
        compareAndSet() {
            throw notImpl('desktopStorage.compareAndSet');
        },
        subscribe() {
            throw notImpl('desktopStorage.subscribe');
        },
    };
}

export const desktopStorage = storage();
