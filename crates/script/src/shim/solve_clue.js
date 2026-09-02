// SolveClue is not v1 — every member throws on use.
const notV1 = (name) => new Error('not v1: ' + name);
const throwUse = (name) => {
    throw notV1(name);
};

export class SolveClue {
    constructor() {
        throwUse('SolveClue');
    }

    validate() {
        throwUse('SolveClue.validate');
    }

    async execute() {
        throwUse('SolveClue.execute');
    }
}

export function heldClueLikeId() {
    throwUse('SolveClue.heldClueLikeId');
}

export function walkToBank() {
    throwUse('SolveClue.walkToBank');
}
