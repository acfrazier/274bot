import assert from 'node:assert/strict';
import test from 'node:test';
import { knifeNpcOperations } from './extractors/knife-npc.ts';
import type { GatheringFacts, MethodWire } from './extractors/gathering.ts';
import type { NpcTransformClosureEvidence } from './extractors/npc-transforms.ts';

function fixture() {
    const names = ['fishing.freshfish.op1', 'fishing.freshfish.op3', 'fishing.0_45_152_lavafish.op1'];
    const methods: MethodWire[] = names.map((id, index) => ({
        id, skill: 'fishing', resources: [], op: { slot: 1, label: 'Lure' },
        targets: { state: 'known', value: [{ kind: 'npc', id: index + 10, op: 1, class: 'resource', respawn: { state: 'known', value: null } }] },
        products: { state: 'known', value: [] }, tools: { state: 'known', value: [] },
        consumes: { state: 'known', value: [] }, requirements: { state: 'known', value: [] },
        spots: { state: 'known', value: null }, forbidden_states: [], sources: [`fishing.rs2:${index + 1}-${index + 1}`],
    }));
    const gathering: GatheringFacts = {
        entities: [], methods, loose: [], zones: [], movements: [], placements: [], hazard_npcs: [], incidental_gem_ids: [],
    };
    const evidence: NpcTransformClosureEvidence = {
        schema: 1, source_inputs: { engine: [], content: [] },
        source_spans: [{ file: 'fishing.rs2', first: 1, last: 3 }],
        npc_types: methods.map((_, index) => ({ npc_type_id: index + 10, category_id: 5, config_state: 'complete', source_refs: [0] })),
        actions: methods.map((_, index) => ({
            npc_type_id: index + 10, slot: 1, state: 'present', label: 'Lure', trigger_code: 9,
            trigger_kind: 'category', trigger_target_id: 5, operation_fingerprint: 'a'.repeat(64), source_refs: [0],
        })),
        transform_edges: [], unresolved: [], proofs: [],
        engine: { complete: true, source_refs: { numeric_dispatch: [0], type_change: [0], reversion: [0], static_type_decoder: [0] } },
    };
    return { gathering, evidence };
}

test('exact knife NPC proof joins reviewed method targets to authored OP/AP handlers, not labels', () => {
    const { gathering, evidence } = fixture();
    evidence.actions[0].label = 'A display label does not define the operation';
    assert.deepEqual(knifeNpcOperations(evidence, gathering), [10, 11, 12].map(npc_type_id => ({
        npc_type_id, slot: 1, operation_fingerprint: 'a'.repeat(64),
    })));
    // A current-type proof is deliberately not a complete transform-closure
    // proof. It does not add any row to the generic numeric-operation allowlist.
    assert.deepEqual(evidence.proofs, []);
});

test('missing, ambiguous or snapshot-label-only exact NPC proofs fail closed', () => {
    const mutations: ((input: { gathering: GatheringFacts; evidence: NpcTransformClosureEvidence }) => void)[] = [
        ({ evidence }) => { evidence.engine.complete = false; },
        ({ evidence }) => { evidence.actions[0].state = 'unknown'; },
        ({ evidence }) => { evidence.actions[0].operation_fingerprint = null; },
        ({ evidence }) => { evidence.actions[0].trigger_code = null; },
        ({ evidence }) => { evidence.actions[0].trigger_kind = 'none'; },
        ({ evidence }) => { evidence.actions[0].source_refs = []; },
        ({ evidence }) => { evidence.npc_types[0].config_state = 'ambiguous'; },
        ({ evidence }) => { evidence.actions.push({ ...evidence.actions[0] }); },
        ({ evidence }) => { evidence.actions[0].slot = 2; },
        ({ gathering }) => { gathering.methods[0].targets = { state: 'unknown', gap: { code: 'missing', sources: [] } }; },
        ({ gathering }) => { gathering.methods[0].sources = ['unrelated.rs2:1-1']; },
        ({ gathering }) => { gathering.methods[0].op = null; },
        ({ gathering }) => { gathering.methods.shift(); },
    ];
    for (const mutate of mutations) {
        const input = fixture();
        mutate(input);
        assert.throws(() => knifeNpcOperations(input.evidence, input.gathering), /knife_npc:/);
    }
});
