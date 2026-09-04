// No-op SolveClue Task: catalog grind bots always `new SolveClue()` in
// onStart (the solveClues flag gates loot/execute inside the class we
// do not have). Constructor must not throw; validate() => false so
// TaskBot never reaches execute. The trail solver is not impl.
const notImpl = (name) => new Error('not impl: ' + name);
const throwUse = (name) => {
    throw notImpl(name);
};

export class SolveClue {
    constructor(_host) {}

    clueStatus() {
        return 'idle';
    }

    noteDeath() {}

    validate() {
        return false;
    }

    async execute() {}
}

export function heldClueLikeId() {
    throwUse('SolveClue.heldClueLikeId');
}

export function walkToBank() {
    throwUse('SolveClue.walkToBank');
}
