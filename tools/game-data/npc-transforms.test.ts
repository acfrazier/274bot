import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

import { runtimeNpcOperations } from './generate.ts';
import { extractNpcTransformClosure, proveNpcTransformSlot } from './extractors/npc-transforms.ts';
import type {
    NpcTransformAction,
    NpcTransformClosureEvidence,
    NpcTransformEdge,
    NpcTransformEngineSourceRefs,
    NpcTransformNpcType,
    NpcTransformProofInput,
    NpcTransformUnresolved,
} from './extractors/npc-transforms.ts';

function writeFixtureFiles(root: string, files: ReadonlyMap<string, string>) {
    for (const [relative, text] of files) {
        const file = path.join(root, relative);
        fs.mkdirSync(path.dirname(file), { recursive: true });
        fs.writeFileSync(file, text);
    }
}

const engineRefs = (): NpcTransformEngineSourceRefs => ({
    numeric_dispatch: [1],
    type_change: [2],
    reversion: [3],
    static_type_decoder: [4],
});

const npcType = (npc_type_id: number): NpcTransformNpcType => ({
    npc_type_id,
    category_id: null,
    config_state: 'complete',
    source_refs: [npc_type_id],
});

const action = (npc_type_id: number, operation_fingerprint = 'same-action'): NpcTransformAction => ({
    npc_type_id,
    slot: 1,
    state: 'present',
    label: 'Talk-to',
    trigger_code: 42,
    trigger_kind: 'type',
    trigger_target_id: npc_type_id,
    operation_fingerprint,
    source_refs: [npc_type_id + 10],
});

const transform = (from_npc_type_id: number | null, to_npc_type_id: number | null, source_refs: number[]): NpcTransformEdge => ({
    from_npc_type_id,
    to_npc_type_id,
    kind: 'scripted',
    change_mode: 'keepall',
    duration_expression: '20',
    reverts_to_base_type: true,
    source_refs,
});

function prove(overrides: Partial<NpcTransformProofInput> = {}) {
    return proveNpcTransformSlot({
        npc_type_id: 101,
        slot: 1,
        npc_types: [npcType(101)],
        actions: [action(101)],
        transform_edges: [],
        unresolved: [],
        engine_complete: true,
        engine_source_refs: engineRefs(),
        ...overrides,
    });
}

const unchanged = prove();
assert.equal(unchanged.status, 'unchanged');
assert.deepEqual(unchanged.reachable_npc_type_ids, [101]);
assert.deepEqual(unchanged.mismatched_npc_type_ids, []);
assert.deepEqual(unchanged.evidence_refs, [1, 111]);

const temporaryClosure = prove({
    npc_types: [npcType(101), npcType(202), npcType(303)],
    actions: [action(101), action(202), action(303)],
    transform_edges: [
        transform(101, 202, [20, 3]),
        transform(202, 303, [21, 3]),
    ],
});
assert.equal(temporaryClosure.status, 'consistent');
assert.deepEqual(temporaryClosure.reachable_npc_type_ids, [101, 202, 303]);
assert.deepEqual(temporaryClosure.mismatched_npc_type_ids, []);
assert.ok(temporaryClosure.evidence_refs.includes(3), 'the base-type reversion path remains part of the proof');

const sameApDifferentOp = prove({
    npc_types: [npcType(101), npcType(202)],
    actions: [
        action(101, 'OP=transform-first;AP=shared-ap'),
        action(202, 'OP=other-handler;AP=shared-ap'),
    ],
    transform_edges: [transform(101, 202, [20])],
});
assert.equal(sameApDifferentOp.status, 'inconsistent', 'the ordered OP/AP fingerprint distinguishes different OP handlers with the same AP handler');

function runtimeEvidence(
    baseFingerprint: string,
    temporaryFingerprint: string,
    proofs: NpcTransformClosureEvidence['proofs'],
): NpcTransformClosureEvidence {
    return {
        schema: 1,
        source_inputs: { engine: [], content: [] },
        source_spans: [],
        npc_types: [npcType(101), npcType(202)],
        actions: [action(101, baseFingerprint), action(202, temporaryFingerprint)],
        transform_edges: [transform(101, 202, [20, 3])],
        unresolved: [],
        proofs,
        engine: { complete: true, source_refs: engineRefs() },
    };
}

const reversionEdge = transform(101, 202, [20, 3]);
const temporaryDifferent = prove({
    npc_type_id: 202,
    npc_types: [npcType(101), npcType(202)],
    actions: [action(101, 'OP=base-handler;AP=shared'), action(202, 'OP=temporary-handler;AP=shared')],
    transform_edges: [reversionEdge],
});
assert.equal(temporaryDifferent.status, 'inconsistent');
assert.deepEqual(temporaryDifferent.reachable_npc_type_ids, [101, 202]);
assert.deepEqual(temporaryDifferent.mismatched_npc_type_ids, [101]);
const refusedRuntime = runtimeNpcOperations(runtimeEvidence('OP=base-handler;AP=shared', 'OP=temporary-handler;AP=shared', [temporaryDifferent]));
assert.ok(!refusedRuntime.operations.some(({ npc_type_id, slot }) => npc_type_id === 202 && slot === 1),
    'a temporary target with a different base handler must not enter runtime operations');

const temporaryMatching = prove({
    npc_type_id: 202,
    npc_types: [npcType(101), npcType(202)],
    actions: [action(101, 'OP=shared;AP=shared'), action(202, 'OP=shared;AP=shared')],
    transform_edges: [reversionEdge],
});
assert.equal(temporaryMatching.status, 'consistent');
assert.deepEqual(temporaryMatching.reachable_npc_type_ids, [101, 202]);
const admittedRuntime = runtimeNpcOperations(runtimeEvidence('OP=shared;AP=shared', 'OP=shared;AP=shared', [temporaryMatching]));
assert.ok(admittedRuntime.operations.some(({ npc_type_id, slot }) => npc_type_id === 202 && slot === 1),
    'a temporary target with the same base handler may enter runtime operations');
assert.deepEqual(sameApDifferentOp.reachable_npc_type_ids, [101, 202]);
assert.deepEqual(sameApDifferentOp.mismatched_npc_type_ids, [202]);

const dynamicUnresolved: NpcTransformUnresolved = {
    scope_npc_type_id: 101,
    scope_slot: null,
    reason: 'dynamic_transform_target',
    expression: 'npc_param(next_npc_type)',
    source_refs: [30],
};
const dynamic = prove({
    transform_edges: [transform(101, null, [20])],
    unresolved: [dynamicUnresolved],
});
assert.equal(dynamic.status, 'unavailable');
assert.deepEqual(dynamic.reachable_npc_type_ids, [101]);
assert.deepEqual(dynamic.unresolved_refs, [0]);

const ambiguousContext = prove({
    npc_types: [npcType(101), npcType(202)],
    actions: [action(101), action(202)],
    transform_edges: [transform(null, 202, [20])],
    unresolved: [{
        scope_npc_type_id: null,
        scope_slot: null,
        reason: 'unknown_transform_context',
        expression: 'proc with source context unavailable',
        source_refs: [31],
    }],
});
assert.equal(ambiguousContext.status, 'unavailable');
assert.deepEqual(ambiguousContext.reachable_npc_type_ids, [101, 202]);
assert.deepEqual(ambiguousContext.unresolved_refs, [0]);

const absentTargetAction = prove({
    npc_types: [npcType(101), npcType(202)],
    actions: [action(101)],
    transform_edges: [transform(101, 202, [20])],
});
assert.equal(absentTargetAction.status, 'inconsistent', 'a complete target config proves the absent slot is a mismatch');
assert.deepEqual(absentTargetAction.mismatched_npc_type_ids, [202]);

const incompleteRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'npc-transform-incomplete-'));
try {
    const engine = path.join(incompleteRoot, 'engine');
    const content = path.join(incompleteRoot, 'content');
    writeFixtureFiles(incompleteRoot, new Map([
        ['engine/src/engine/entity/Npc.ts', 'export default class Npc {}\n'],
        ['engine/src/engine/script/ServerTriggerType.ts', 'APNPC1 = 3,\nOPNPC1 = 10,\n'],
        ['content/pack/npc.pack', '101=test_npc\n'],
        ['content/pack/category.pack', '1=test_category\n'],
        ['content/pack/param.pack', '1=test_param\n'],
        ['content/pack/script.pack', '1=test_script\n'],
        ['content/scripts/test.npc', '[test_npc]\nop1=Talk-to\n'],
        ['content/scripts/test.rs2', '[apnpc1,_]\nreturn;\n'],
    ]));
    const evidence = extractNpcTransformClosure(engine, content);
    assert.deepEqual(evidence.npc_types.map(({ npc_type_id, category_id }) => ({ npc_type_id, category_id })), [{ npc_type_id: 101, category_id: null }]);
    assert.equal(evidence.actions[0].npc_type_id, 101);
    assert.equal(evidence.actions[0].trigger_kind, 'global', 'an underscore subject resolves as a global handler');
    assert.equal(evidence.proofs[0].status, 'unavailable', 'partial engine evidence must not authorize the extracted operation');
} finally {
    fs.rmSync(incompleteRoot, { recursive: true, force: true });
}

const extractionRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'npc-transform-extract-'));
try {
    const engine = path.join(extractionRoot, 'engine');
    const content = path.join(extractionRoot, 'content');
    writeFixtureFiles(extractionRoot, new Map([
        ['engine/src/cache/config/NpcType.ts', `
if (code === 18) return;
if (code >= 30 && code < 40) this.op[code - 30] = dat.gjstr();
category = -1;
throw new Error('Unrecognized npc config code');
`],
        ['engine/src/engine/entity/Npc.ts', `
class Npc {
    changeType(type: number, duration: number) {
        this.type = type;
        this.type = type;
        this.baseType = type;
        this.resetOnRevert = reset;
        this.setLifeCycle(duration);
        this.revertType();
    }
    private revertType() {
        this.type = this.baseType;
        this.type = this.baseType;
        World.removeNpc(this, -1);
        World.addNpc(this, -1, false);
    }
}
`],
        ['engine/src/engine/World.ts', 'npc.resetEntity(true);\n'],
        ['engine/src/engine/script/ScriptOpcode.ts', 'NPC_CHANGETYPE,\nNPC_CHANGETYPE_KEEPALL,\n'],
        ['engine/src/engine/script/handlers/NpcOps.ts', `
const handlers = {
    [ScriptOpcode.NPC_CHANGETYPE]: () => state.activeNpc.changeType(npcType, duration),
    [ScriptOpcode.NPC_CHANGETYPE_KEEPALL]: () => state.activeNpc.changeType(npcType, duration, false),
};
`],
        ['engine/src/engine/script/ScriptProvider.ts', `
    static getByTrigger(trigger: number, type: number, category: number) {
        let script = ScriptProvider.scriptLookup.get(trigger | (0x2 << 8) | (type << 10));
        if (script) {
            return script;
        }
        script = ScriptProvider.scriptLookup.get(trigger | (0x1 << 8) | (category << 10));
        if (script) {
            return script;
        }
        return ScriptProvider.scriptLookup.get(trigger);
    }
`],
        ['engine/src/engine/script/ServerTriggerType.ts', `
APNPC1 = 3,
APNPC2 = 4,
APNPC3 = 5,
APNPC4 = 6,
OPNPC1 = 10,
OPNPC2 = 11,
OPNPC3 = 12,
OPNPC4 = 13,
`],
        ['engine/src/engine/entity/Player.ts', `
class Player {
    getOpTrigger() {
        return ScriptProvider.getByTrigger(this.targetOp + 7, typeId, categoryId) ?? null;
    }
    getApTrigger() {
        return ScriptProvider.getByTrigger(this.targetOp, typeId, categoryId) ?? null;
    }
    tryInteract(allowOpScenery: boolean) {
        const opTrigger = this.getOpTrigger();
        const apTrigger = this.getApTrigger();
        if (opTrigger && this.inOperableDistance(this.target)) {
            run(opTrigger);
        } else if (apTrigger && this.inApproachDistance(this.apRange, this.target)) {
            run(apTrigger);
        }
    }
}
`],
        ['engine/src/network/game/client/handler/OpNpcHandler.ts', `
const npcType = NpcType.get(npc.type);
if (npcType.op[message.op - 1] === null) return;
if (npcType.op[message.op - 1] === 'hidden') return;
const trigger: ServerTriggerType = ServerTriggerType.APNPC1 + (message.op - 1);
`],
        ['content/pack/npc.pack', '101=test_npc\n202=transformed_npc\n303=global_npc\n404=op_only_npc\n505=barbarian\n606=category_npc\n707=category_only_npc\n808=no_handler_npc\n'],
        ['content/pack/category.pack', '1=test_category\n2=barbarian\n3=only\n'],
        ['content/pack/param.pack', '1=test_param\n'],
        ['content/pack/script.pack', '1=test_script\n'],
        ['content/scripts/test.npc', `
 [test_npc]
 op1=Talk-to
 [transformed_npc]
 op1=Talk-to
 [global_npc]
 op2=Use
 [op_only_npc]
 op3=Talk-to
 [barbarian]
 op1=Talk-to
 category=barbarian
 [category_npc]
 op3=Talk-to
 category=only
 [category_only_npc]
 op4=Talk-to
 [no_handler_npc]
 op3=Talk-to
 `],
        ['content/scripts/handlers.rs2', `
[opnpc1,test_npc]
npc_changetype(202, 1000)
[opnpc1,transformed_npc]
return;
[apnpc1,_]
return;
[apnpc2,_]
return;
[opnpc3,op_only_npc]
return;
[opnpc1,barbarian]
return;
[opnpc1,_barbarian]
return;
[opnpc4,only]
return;
[opnpc4,category_only]
return;
[opnpc3,_only]
return;
`],
    ]));
    const evidence = extractNpcTransformClosure(engine, content);
    assert.equal(evidence.engine.complete, true);
    const actionByPair = new Map(evidence.actions.map((row) => [`${row.npc_type_id}:${row.slot}`, row]));
    const base = actionByPair.get('101:1')!;
    const transformed = actionByPair.get('202:1')!;
    assert.equal(base.state, 'present');
    assert.equal(transformed.state, 'present');
    assert.notEqual(base.operation_fingerprint, transformed.operation_fingerprint, 'same AP behavior with different OP behavior must have different pair fingerprints');
    const transformedProof = evidence.proofs.find(({ npc_type_id, slot }) => npc_type_id === 101 && slot === 1)!;
    assert.equal(transformedProof.status, 'inconsistent');
    assert.deepEqual(transformedProof.reachable_npc_type_ids, [101, 202]);
    assert.deepEqual(transformedProof.mismatched_npc_type_ids, [202]);
    const temporaryTargetProof = evidence.proofs.find(({ npc_type_id, slot }) => npc_type_id === 202 && slot === 1)!;
    assert.equal(temporaryTargetProof.status, 'inconsistent', 'the extracted temporary form must include its different base handler');
    assert.deepEqual(temporaryTargetProof.reachable_npc_type_ids, [101, 202]);
    assert.ok(!runtimeNpcOperations(evidence).operations.some(({ npc_type_id, slot }) => npc_type_id === 202 && slot === 1));
    const sharedApHeader = evidence.source_spans.findIndex(({ file, first }) => file === 'scripts/handlers.rs2' && first === 6);
    assert.ok(sharedApHeader >= 0);
    assert.ok(base.source_refs.includes(sharedApHeader));
    assert.ok(transformed.source_refs.includes(sharedApHeader), 'both types resolve the same global AP handler');

    const barbarian = actionByPair.get('505:1')!;
    assert.equal(barbarian.trigger_kind, 'type', 'a bare NPC/category name collision resolves to the NPC type');
    assert.equal(barbarian.trigger_target_id, 505);
    const categoryNpc = actionByPair.get('606:3')!;
    assert.equal(categoryNpc.trigger_kind, 'category', 'only an underscore-prefixed subject selects a category');
    assert.equal(categoryNpc.trigger_target_id, 3);
    assert.ok(evidence.unresolved.some(({ reason, source_refs }) =>
        reason === 'unknown_trigger_target'
        && [16, 18].every(line => source_refs.some(ref => evidence.source_spans[ref]?.first === line)),
    ), 'category-only bare names and category_ aliases remain unresolved');
    const noHandler = actionByPair.get('808:3')!;
    assert.equal(noHandler.state, 'unknown', 'a configured option with no OP/AP handler has no comparable action');
    assert.equal(noHandler.trigger_kind, 'none', 'a configured option with no handler names no trigger target');
    assert.ok(evidence.unresolved.some(({ scope_npc_type_id, scope_slot, reason }) =>
        scope_npc_type_id === 808 && scope_slot === 3 && reason === 'missing_trigger_handler',
    ), 'a configured option with no handler reports missing_trigger_handler, not an unknown subject');
    assert.equal(evidence.proofs.find(({ npc_type_id, slot }) => npc_type_id === 808 && slot === 3)?.status, 'unavailable');
    assert.ok(!runtimeNpcOperations(evidence).operations.some(({ npc_type_id, slot }) => npc_type_id === 808 && slot === 3),
        'an option with no handler must not enter runtime operations');
    const opOnly = actionByPair.get('404:3')!;
    assert.equal(opOnly.state, 'present');
    assert.equal(opOnly.trigger_kind, 'type');
    assert.ok(opOnly.operation_fingerprint, 'an OPNPC-only slot has a complete handler fingerprint');
    assert.equal(evidence.proofs.find(({ npc_type_id, slot }) => npc_type_id === 404 && slot === 3)?.status, 'unchanged');

    const globalAp = actionByPair.get('303:2')!;
    assert.equal(globalAp.state, 'present');
    assert.equal(globalAp.trigger_kind, 'global', '[apnpc2,_] resolves as the slot-wide global handler');
    assert.equal(globalAp.trigger_target_id, null);
    assert.ok(globalAp.operation_fingerprint);
    assert.equal(evidence.proofs.find(({ npc_type_id, slot }) => npc_type_id === 303 && slot === 2)?.status, 'unchanged');

    const numericDispatchSpans = evidence.engine.source_refs.numeric_dispatch.map((ref) => evidence.source_spans[ref]);
    const playerDispatchLines = numericDispatchSpans
        .filter(({ file }) => file === 'src/engine/entity/Player.ts')
        .map(({ first }) => first)
        .sort((left, right) => left - right);
    assert.deepEqual(playerDispatchLines, [4, 7, 10, 11, 12, 14], 'the proof cites OP lookup, AP fallback, and their engine order');
    assert.ok(numericDispatchSpans.some(({ file }) => file === 'src/engine/script/ScriptProvider.ts'), 'the proof cites type, category, and global trigger precedence');
} finally {
    fs.rmSync(extractionRoot, { recursive: true, force: true });
}

console.log('npc transform slot proof regressions passed');
