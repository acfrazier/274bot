import { notImpl } from '../../shim/_kernel.js';

export function combatKeepNames(o) {
    const fn = globalThis.rustyscript.functions.__rs2b0t_combat_keep_names;
    if (typeof fn !== 'function') throw notImpl('combatKeepNames');
    return fn(o && typeof o === 'object' ? o : {});
}
