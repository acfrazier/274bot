export function resolveWorldNumber(host, params) {
    return globalThis.rustyscript.functions.__rs2b0t_resolve_world_number(
        host,
        params.getAll('world'),
    );
}
