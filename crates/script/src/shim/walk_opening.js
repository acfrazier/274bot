import { notImpl, runMachine } from '../../shim/_kernel.js';

export function faceSteps(door, canStep) {
    return globalThis.__rs2b0t_walk_opening('faceSteps', door, canStep);
}

export function openedNeighbour(door, before, after) {
    return globalThis.__rs2b0t_walk_opening('openedNeighbour', door, before, after);
}

export function farSide(door, across, here) {
    return globalThis.__rs2b0t_walk_opening('farSide', door, across, here);
}

export function isOpenableObstacle(name, actions, obstacles) {
    return globalThis.__rs2b0t_walk_opening('isOpenableObstacle', name, actions, obstacles);
}

export function openOp(actions) {
    return globalThis.__rs2b0t_walk_opening('openOp', actions);
}

export function closeOp(actions) {
    return globalThis.__rs2b0t_walk_opening('closeOp', actions);
}

export function towardDest(door, here, dest) {
    return globalThis.__rs2b0t_walk_opening('towardDest', door, here, dest);
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
