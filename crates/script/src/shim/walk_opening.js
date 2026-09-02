import { Game } from '../../api/game/Game.js';
import { Traversal } from '../../api/walking/Traversal.js';

export function openOp(actions) {
    return (actions || []).find((a) => /^open/i.test(String(a))) ?? null;
}

export function towardDest(from, here, toward) {
    const dx = toward.x - here.x;
    const dz = toward.z - here.z;
    const dot = dx * (from.x - here.x) + dz * (from.z - here.z);
    return dot > 0;
}

export function isOpenableObstacle(name, actions, obstacles) {
    const n = (name ?? '').toLowerCase();
    return obstacles.some((k) => n.includes(k)) && (actions || []).some((a) => /^open/i.test(a));
}

export async function walkOpening(dest, radius, _obstacles, log) {
    await Traversal.walkTo(dest, { radius, timeoutMs: 90_000, log: (m) => log?.(m) });
    const here = Game.tile();
    if (!here || !dest) {
        return false;
    }
    const cheb = Math.max(Math.abs(here.x - dest.x), Math.abs(here.z - dest.z));
    return cheb <= radius;
}
