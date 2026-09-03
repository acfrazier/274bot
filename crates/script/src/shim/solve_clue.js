// SolveClue is not impl — every member throws on use.
const notImpl = (name) => new Error('not impl: ' + name);
const throwUse = (name) => {
    throw notImpl(name);
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
