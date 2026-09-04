import { snap } from '../../shim/_kernel.js';

export function combatKeepNames(o) {
    const names = [];
    for (const r of snap().equipment || []) {
        if (r && r.name) names.push(r.name);
    }
    for (const r of snap().inv || []) {
        if (r && r.name) names.push(r.name);
    }
    if (o && Array.isArray(o.extra)) names.push(...o.extra);
    return names;
}
