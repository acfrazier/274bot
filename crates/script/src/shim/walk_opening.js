import { notImpl, runMachine } from '../../shim/_kernel.js';

export function openOp(actions) {
    return (actions || []).find((a) => /^open/i.test(String(a))) ?? null;
}

export function towardDest(_from, _here, _toward) {
    throw notImpl('towardDest');
}

export function isOpenableObstacle(_name, _actions, _obstacles) {
    throw notImpl('isOpenableObstacle');
}

export async function walkOpening(dest, radius, obstacles, log) {
    const out = await runMachine(
        'walk-opening',
        {
            dest: { x: dest.x, z: dest.z, level: dest.level ?? 0 },
            radius: radius ?? 0,
            obstacles: Array.isArray(obstacles) ? obstacles : [],
        },
        { log: typeof log === 'function' ? log : undefined },
    );
    if (out.kind === 'refused') throw notImpl('walkOpening', out.reason);
    return out.kind === 'done' && out.value === true;
}
