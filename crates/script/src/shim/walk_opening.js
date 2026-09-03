import { Traversal } from '../../api/walking/Traversal.js';
import { notV1 } from '../../shim/_kernel.js';

export function openOp(actions) {
    return (actions || []).find((a) => /^open/i.test(String(a))) ?? null;
}

export function towardDest(_from, _here, _toward) {
    throw notV1('towardDest');
}

export function isOpenableObstacle(_name, _actions, _obstacles) {
    throw notV1('isOpenableObstacle');
}

export async function walkOpening(dest, radius, _obstacles, log) {
    return Traversal.walkResilient(dest, { radius, timeoutMs: 90_000, log: (m) => log?.(m) });
}
