import { notImpl, notImplValue } from '../../../../shim/_kernel.js';

// The ClueSolver card only consumes these immutable catalog names directly.
// Acquisition policy remains in the Rust clue machine.
export const SPADE_NAME = 'Spade';
export const TRIO = Object.freeze(['Sextant', 'Watch', 'Chart']);
export const SPADE_SPAWNS = notImplValue('SPADE_SPAWNS');
export const PROFESSOR = notImplValue('PROFESSOR');
export const MURPHY = notImplValue('MURPHY');
export const KOJO = notImplValue('KOJO');
export const KOJO_EXIT = notImplValue('KOJO_EXIT');

export function trailKit(scrollId) {
    return globalThis.rustyscript.functions.__rs2b0t_clue({ op: 'trailKit', scrollId });
}

export function nextCoordTool(held) {
    return globalThis.rustyscript.functions.__rs2b0t_clue({ op: 'nextCoordTool', held });
}

export function extraItemShop(name) {
    throw notImpl('extraItemShop');
}

export function gateItemShop(name) {
    throw notImpl('gateItemShop');
}
