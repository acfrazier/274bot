const selected = (...args) => globalThis.__rs2b0t_selected_facts('world-number', ...args);

export function resolveWorldNumber(host, params) {
    const values = params.getAll('world');
    return selected(host, values.length, values[0]);
}
