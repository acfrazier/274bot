import type { GatheringFacts } from './gathering.ts';
import type { NpcTransformClosureEvidence } from './npc-transforms.ts';

export type KnifeNpcOperation = { npc_type_id: number; slot: number; operation_fingerprint: string };

// Only these reviewed continuing fishing methods put an NPC click in KnifeArm.
// Ending fishing uses a separate ordinary checked click after the arm.
const CONTINUING_NPC_METHODS = [
    'fishing.freshfish.op1',
    'fishing.freshfish.op3',
    'fishing.0_45_152_lavafish.op1',
] as const;

/**
 * Prove the selected current type's authored numeric operation, not transform
 * closure. KnifeArm binds index, current type, tile and NPC-info generation at
 * dispatch. Generic numeric NPC admission still requires the complete transform
 * proof. Menu labels alone never admit an operation through either path.
 */
export function knifeNpcOperations(evidence: NpcTransformClosureEvidence, gathering: GatheringFacts): KnifeNpcOperation[] {
    if (!evidence.engine.complete) throw new Error('knife_npc: numeric NPC dispatch is not proved');
    const operations: KnifeNpcOperation[] = [];
    for (const id of CONTINUING_NPC_METHODS) {
        const methods = gathering.methods.filter(method => method.id === id);
        if (methods.length !== 1) throw new Error(`knife_npc: ${id} does not have one method`);
        const method = methods[0];
        if (method.targets.state !== 'known') throw new Error(`knife_npc: ${id} targets unavailable`);
        const targets = method.targets.value.filter(target => target.kind === 'npc' && target.class === 'resource');
        if (targets.length === 0) throw new Error(`knife_npc: ${id} has no NPC resource target`);
        for (const target of targets) {
            const types = evidence.npc_types.filter(type => type.npc_type_id === target.id);
            const actions = evidence.actions.filter(action => action.npc_type_id === target.id && action.slot === target.op);
            const action = actions[0];
            const handlerMatches = action?.source_refs.some(ref => {
                const source = evidence.source_spans[ref];
                return source && method.sources.some(citation => {
                    const parsed = /^(.*):(\d+)-(\d+)$/.exec(citation);
                    return parsed && source.file === parsed[1]
                        && source.first <= Number(parsed[2]) && source.last >= Number(parsed[3]);
                });
            });
            if (types.length !== 1 || types[0].config_state !== 'complete'
                || actions.length !== 1 || action.state !== 'present'
                || action.trigger_code === null || !['type', 'category', 'global'].includes(action.trigger_kind)
                || !action.operation_fingerprint || !/^[0-9a-f]{64}$/.test(action.operation_fingerprint)
                || !handlerMatches || method.op?.slot !== target.op) {
                throw new Error(`knife_npc: ${id} target ${target.id}/${target.op} lacks a positive content handler proof`);
            }
            operations.push({ npc_type_id: target.id, slot: target.op, operation_fingerprint: action.operation_fingerprint });
        }
    }
    operations.sort((left, right) => left.npc_type_id - right.npc_type_id || left.slot - right.slot);
    if (operations.some((row, index) => index > 0
        && row.npc_type_id === operations[index - 1].npc_type_id && row.slot === operations[index - 1].slot)) {
        throw new Error('knife_npc: duplicate exact NPC operation');
    }
    return operations;
}
