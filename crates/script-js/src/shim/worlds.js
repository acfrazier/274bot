const selected = (...args) => globalThis.__rs2b0t_selected_facts('world-number', ...args);

export function resolveWorldNumber(host, params) {
    const hostname = new URL(`https://${host}`).hostname;
    const values = params.getAll('world');
    return selected(hostname, values.length, values[0]);
}
