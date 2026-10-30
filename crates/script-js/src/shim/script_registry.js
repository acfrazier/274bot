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
