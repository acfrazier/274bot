import { defineBot } from './defineBot.js';

const call = (op, value) => globalThis.__rs2b0t_script_registry(op, value);

export const ScriptRegistry = {
    register(meta) {
        call('register', meta);
    },
    list() {
        return call('list');
    },
    get(name) {
        return call('get', name);
    },
};

export function registerScript(manifest, origin) {
    return globalThis.__rs2b0t_script_registry('registerScript', manifest, origin, defineBot);
}
