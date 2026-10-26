import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { parsePack, walkContentFiles } from './common.ts';
import { parseRs2Blocks, stripComment } from './gathering-content.ts';

/** Engine sources whose semantics define numeric NPC dispatch and type changes. */
export const NPC_TRANSFORM_ENGINE_INPUT_PATHS = [
    'src/cache/config/NpcType.ts',
    'src/engine/entity/Npc.ts',
    'src/engine/World.ts',
    'src/engine/script/handlers/NpcOps.ts',
    'src/engine/script/ScriptOpcode.ts',
    'src/engine/script/ScriptProvider.ts',
    'src/engine/script/ServerTriggerType.ts',
    'src/network/game/client/handler/OpNpcHandler.ts',
    'src/engine/entity/Player.ts',
] as const;

/** Content packs used to resolve numeric NPC and category identities. */
export const NPC_TRANSFORM_CONTENT_PACK_INPUT_PATHS = [
    'pack/category.pack',
    'pack/npc.pack',
    'pack/param.pack',
    'pack/script.pack',
] as const;

export type SourceSpan = { file: string; first: number; last: number };
export type SourceInputDigest = { path: string; bytes: number; sha256: string };
export type NpcTransformTypeState = 'complete' | 'missing' | 'ambiguous';
export type NpcTransformTriggerKind = 'type' | 'category' | 'global' | 'unresolved' | 'none';
export type NpcTransformActionState = 'present' | 'unknown';
export type NpcTransformStatus = 'unchanged' | 'consistent' | 'inconsistent' | 'unavailable';
export type NpcTransformEdgeKind = 'scripted' | 'npc_param';
export type NpcTransformChangeMode = 'reset' | 'keepall';
export type NpcTransformUnresolvedReason =
    | 'dynamic_transform_target'
    | 'unknown_transform_context'
    | 'ambiguous_npc_config'
    | 'missing_npc_config'
    | 'unknown_trigger_target'
    | 'missing_trigger_handler'
    | 'ambiguous_trigger'
    | 'unknown_category'
    | 'unsupported_transform_expression'
    | 'engine_source_incomplete'
    | 'unknown_engine_mutator'
    | 'missing_selected_input';

export type NpcTransformNpcType = {
    npc_type_id: number;
    category_id: number | null;
    config_state: NpcTransformTypeState;
    source_refs: number[];
};

/** An authored numeric NPC option and its source-proven ordered OP/AP handler pair. */
export type NpcTransformAction = {
    npc_type_id: number;
    slot: number;
    state: NpcTransformActionState;
    /** Authored menu text. This field is never used as proof of action identity. */
    label: string | null;
    /** APNPC code assigned to this client option. */
    trigger_code: number | null;
    /** Preferred handler target class, with OPNPC before APNPC. */
    trigger_kind: NpcTransformTriggerKind;
    /** Numeric type or category target of the preferred handler, when applicable. */
    trigger_target_id: number | null;
    /** Hash of canonical OPNPC-then-APNPC handler behavior, not of the menu label. */
    operation_fingerprint: string | null;
    source_refs: number[];
};

/** One source-proven scripted type change. Reversion returns to the NPC's original base type. */
export type NpcTransformEdge = {
    from_npc_type_id: number | null;
    to_npc_type_id: number | null;
    kind: NpcTransformEdgeKind;
    change_mode: NpcTransformChangeMode;
    duration_expression: string;
    reverts_to_base_type: true;
    source_refs: number[];
};

/** A source condition that prevents a complete numeric-slot proof. */
export type NpcTransformUnresolved = {
    scope_npc_type_id: number | null;
    scope_slot: number | null;
    reason: NpcTransformUnresolvedReason;
    expression: string | null;
    source_refs: number[];
};

/** A proof row for one source-derived, non-hidden numeric NPC option. */
export type NpcTransformProof = {
    npc_type_id: number;
    slot: number;
    status: NpcTransformStatus;
    reachable_npc_type_ids: number[];
    mismatched_npc_type_ids: number[];
    evidence_refs: number[];
    unresolved_refs: number[];
};

export type NpcTransformEngineSourceRefs = {
    numeric_dispatch: number[];
    type_change: number[];
    reversion: number[];
    static_type_decoder: number[];
};

export type NpcTransformClosureEvidence = {
    schema: 1;
    source_inputs: { engine: SourceInputDigest[]; content: SourceInputDigest[] };
    source_spans: SourceSpan[];
    npc_types: NpcTransformNpcType[];
    /** Sorted by (npc_type_id, slot). Only authored, non-hidden options are listed. */
    actions: NpcTransformAction[];
    /** Sorted by source type, target type, kind, mode, duration and source refs. */
    transform_edges: NpcTransformEdge[];
    unresolved: NpcTransformUnresolved[];
    /** Sorted by (npc_type_id, slot), for every listed action. */
    proofs: NpcTransformProof[];
    engine: { complete: boolean; source_refs: NpcTransformEngineSourceRefs };
};

export type NpcTransformProofInput = {
    npc_type_id: number;
    slot: number;
    npc_types: readonly NpcTransformNpcType[];
    actions: readonly NpcTransformAction[];
    transform_edges: readonly NpcTransformEdge[];
    unresolved: readonly NpcTransformUnresolved[];
    engine_complete: boolean;
    engine_source_refs: NpcTransformEngineSourceRefs;
};

export type NpcTransformInputPaths = { engine: string[]; content: string[] };

const NPC_PARAM = /^npc_param\s*\(\s*([A-Za-z0-9_]+)\s*\)$/i;
const INTEGER = /^\d+$/;

function normalizeEngineRoot(engineRoot: string) {
    const repositorySource = path.join(engineRoot, 'src');
    if (fs.existsSync(path.join(repositorySource, 'engine/entity/Npc.ts'))) {
        return { root: engineRoot, source: repositorySource };
    }
    if (fs.existsSync(path.join(engineRoot, 'engine/entity/Npc.ts'))) {
        return { root: engineRoot, source: engineRoot };
    }
    throw new Error(`npc_transforms: engine source tree not found below ${engineRoot}`);
}

function sortedUnique(values: string[]) {
    return [...new Set(values)].sort((a, b) => a < b ? -1 : a > b ? 1 : 0);
}

/** Return the exact root-relative inventory read by the extractor. */
export function npcTransformInputPaths(engineRoot: string, contentRoot: string): NpcTransformInputPaths {
    const engine = normalizeEngineRoot(engineRoot);
    const engineFiles = walkContentFiles(engine.source, '.ts').map((file) => path.relative(engine.root, file).split(path.sep).join('/'));
    const scriptsRoot = path.join(contentRoot, 'scripts');
    if (!fs.existsSync(scriptsRoot)) throw new Error(`npc_transforms: scripts directory missing below ${contentRoot}`);
    const scripts = [
        ...walkContentFiles(scriptsRoot, '.npc'),
        ...walkContentFiles(scriptsRoot, '.rs2'),
    ].map((file) => path.relative(contentRoot, file).split(path.sep).join('/'));
    const packs = NPC_TRANSFORM_CONTENT_PACK_INPUT_PATHS.map((file) => {
        if (!fs.existsSync(path.join(contentRoot, file))) throw new Error(`npc_transforms: required input missing: ${file}`);
        return file;
    });
    return { engine: sortedUnique(engineFiles), content: sortedUnique([...scripts, ...packs]) };
}

function hashText(text: string) {
    return crypto.createHash('sha256').update(text).digest('hex');
}

function readInputs(root: string, files: readonly string[]) {
    const texts = new Map<string, string>();
    const hashes: SourceInputDigest[] = [];
    for (const relative of files) {
        const absolute = path.join(root, relative);
        const bytes = fs.readFileSync(absolute);
        const text = bytes.toString('utf8');
        texts.set(relative, text);
        hashes.push({ path: relative, bytes: bytes.length, sha256: hashTextBytes(bytes) });
    }
    return { texts, hashes };
}

function hashTextBytes(bytes: Buffer) {
    return crypto.createHash('sha256').update(bytes).digest('hex');
}

class SourceRegistry {
    readonly spans: SourceSpan[] = [];
    private readonly byKey = new Map<string, number>();

    add(span: SourceSpan): number {
        const key = `${span.file}\0${span.first}\0${span.last}`;
        const existing = this.byKey.get(key);
        if (existing !== undefined) return existing;
        const index = this.spans.length;
        this.spans.push(span);
        this.byKey.set(key, index);
        return index;
    }
}

function uniqueRefs(refs: readonly number[]) {
    return [...new Set(refs)].sort((a, b) => a - b);
}

function lineSpan(file: string, line: number): SourceSpan {
    return { file, first: line, last: line };
}

type PackIdentity = { id: number; name: string; source: SourceSpan };
type StrictPack = {
    byName: Map<string, PackIdentity>;
    byId: Map<number, PackIdentity>;
    entries: PackIdentity[];
};

function parseStrictPack(file: string, text: string): StrictPack {
    const parsed = parsePack(text);
    const byName = new Map<string, PackIdentity>();
    const byId = new Map<number, PackIdentity>();
    const entries: PackIdentity[] = [];
    text.split(/\r?\n/).forEach((raw, index) => {
        const value = raw.trim();
        if (!value || value.startsWith('//')) return;
        const equals = value.indexOf('=');
        if (equals <= 0) throw new Error(`${file}:${index + 1}: invalid packed identity row`);
        const rawId = value.slice(0, equals);
        const name = value.slice(equals + 1);
        const id = Number(rawId);
        if (!/^\d+$/.test(rawId) || !Number.isSafeInteger(id) || !name) throw new Error(`${file}:${index + 1}: invalid packed identity ${value}`);
        if (byName.has(name) || byId.has(id)) throw new Error(`${file}:${index + 1}: ambiguous packed identity ${name}/${id}`);
        if (parsed.get(name) !== id) throw new Error(`${file}:${index + 1}: pack parser disagreement for ${name}`);
        const entry = { id, name, source: lineSpan(file, index + 1) };
        byName.set(name, entry);
        byId.set(id, entry);
        entries.push(entry);
    });
    if (entries.length === 0) throw new Error(`${file}: empty packed identity table`);
    return { byName, byId, entries };
}

type NpcConfigFact = {
    name: string;
    source: SourceSpan;
    operations: Map<number, { value: string; source: SourceSpan }[]>;
    categories: { value: string; source: SourceSpan }[];
    params: Map<string, { value: string; source: SourceSpan }[]>;
};

function configValueLine(raw: string) {
    let quoted = false;
    let escaped = false;
    for (let i = 0; i < raw.length - 1; i += 1) {
        const ch = raw[i];
        if (escaped) escaped = false;
        else if (ch === '\\' && quoted) escaped = true;
        else if (ch === '"') quoted = !quoted;
        else if (!quoted && ch === '/' && raw[i + 1] === '/') return raw.slice(0, i).trim();
    }
    return raw.trim();
}

function parseNpcConfigFile(file: string, text: string) {
    const out: NpcConfigFact[] = [];
    let current: NpcConfigFact | null = null;
    text.split(/\r?\n/).forEach((raw, index) => {
        const line = configValueLine(raw);
        if (!line) return;
        const number = index + 1;
        if (line.startsWith('[') && line.endsWith(']')) {
            const name = line.slice(1, -1).trim();
            if (!name) throw new Error(`${file}:${number}: empty NPC config name`);
            current = { name, source: lineSpan(file, number), operations: new Map(), categories: [], params: new Map() };
            out.push(current);
            return;
        }
        if (!current) return;
        current.source.last = number;
        const equals = line.indexOf('=');
        if (equals <= 0) return;
        const key = line.slice(0, equals).trim();
        const value = line.slice(equals + 1).trim();
        const op = /^op(\d+)$/.exec(key);
        if (op) {
            const slot = Number(op[1]);
            if (!Number.isSafeInteger(slot) || slot < 1) throw new Error(`${file}:${number}: invalid NPC option slot ${key}`);
            (current.operations.get(slot) ?? current.operations.set(slot, []).get(slot)!).push({ value, source: lineSpan(file, number) });
        } else if (key === 'category') {
            current.categories.push({ value, source: lineSpan(file, number) });
        } else if (key === 'param') {
            const comma = value.indexOf(',');
            if (comma <= 0) return;
            const name = value.slice(0, comma).trim();
            const paramValue = value.slice(comma + 1).trim();
            (current.params.get(name) ?? current.params.set(name, []).get(name)!).push({ value: paramValue, source: lineSpan(file, number) });
        }
    });
    return out;
}

type NpcTypeFact = {
    npc_type_id: number;
    name: string;
    pack_source: SourceSpan;
    config_state: NpcTransformTypeState;
    config: NpcConfigFact | null;
    category_id: number | null;
    category_known: boolean;
};

function sameValues(values: readonly { value: string }[]) {
    return values.length > 0 && values.every((item) => item.value === values[0].value);
}

function parseTriggerCodes(text: string) {
    const ap = new Map<number, number>();
    const op = new Map<number, number>();
    const seenCodes = new Set<number>();
    let complete = true;
    text.split(/\r?\n/).forEach((line) => {
        const sourceLine = stripComment(line);
        const match = /^\s*(AP|OP)NPC(\d+)\s*=\s*(\d+)\s*,?\s*$/.exec(sourceLine);
        if (!match) {
            if (/^\s*(?:AP|OP)NPC\d+\s*=/.test(sourceLine)) complete = false;
            return;
        }
        const codes = match[1] === 'AP' ? ap : op;
        const slot = Number(match[2]);
        const code = Number(match[3]);
        if (!Number.isSafeInteger(slot) || slot < 1 || !Number.isSafeInteger(code) || codes.has(slot) || seenCodes.has(code)) {
            complete = false;
            return;
        }
        codes.set(slot, code);
        seenCodes.add(code);
    });
    return { ap, op, complete };
}

function findLine(file: string, text: string, expression: RegExp): SourceSpan[] {
    const out: SourceSpan[] = [];
    text.split(/\r?\n/).forEach((line, index) => {
        expression.lastIndex = 0;
        if (expression.test(line)) out.push(lineSpan(file, index + 1));
    });
    return out;
}

function engineSemantics(engineFiles: ReadonlyMap<string, string>, sources: SourceRegistry) {
    const sourceRefs: NpcTransformEngineSourceRefs = { numeric_dispatch: [], type_change: [], reversion: [], static_type_decoder: [] };
    const required = new Set<string>(NPC_TRANSFORM_ENGINE_INPUT_PATHS);
    const files = new Map<string, { relative: string; text: string }>();
    for (const [relative, text] of engineFiles) {
        const logical = relative.startsWith('src/') ? relative.slice('src/'.length) : relative;
        files.set(logical, { relative, text });
    }
    const find = (logical: string, expression: RegExp) => {
        const file = files.get(logical);
        return file ? findLine(file.relative, file.text, expression) : [];
    };
    const text = (logical: string) => files.get(logical)?.text ?? '';

    const npcTypeFile = 'cache/config/NpcType.ts';
    const npcTypeText = text(npcTypeFile);
    sourceRefs.static_type_decoder = find(npcTypeFile, /code === 18|code >= 30 && code < 40|this\.op\[code - 30\]|category = -1;|Unrecognized npc config code/).map((span) => sources.add(span));

    const opHandlerFile = 'network/game/client/handler/OpNpcHandler.ts';
    const opHandlerText = text(opHandlerFile);
    sourceRefs.numeric_dispatch = [
        ...find(opHandlerFile, /npc\.type/),
        ...find(opHandlerFile, /npcType\.op\[message\.op - 1\]/),
        ...find(opHandlerFile, /ServerTriggerType\.APNPC1 \+ \(message\.op - 1\)/),
    ].map((span) => sources.add(span));
    const playerFile = 'engine/entity/Player.ts';
    const playerText = text(playerFile);
    sourceRefs.numeric_dispatch.push(...[
        ...find(playerFile, /return ScriptProvider\.getByTrigger\(this\.targetOp \+ 7, typeId, categoryId\)/),
        ...find(playerFile, /return ScriptProvider\.getByTrigger\(this\.targetOp, typeId, categoryId\)/),
        ...find(playerFile, /const opTrigger = this\.getOpTrigger\(\)/),
        ...find(playerFile, /const apTrigger = this\.getApTrigger\(\)/),
        ...find(playerFile, /if \(opTrigger &&/),
        ...find(playerFile, /else if \(apTrigger &&/),
    ].map((span) => sources.add(span)));

    const opcodeFile = 'engine/script/ScriptOpcode.ts';
    const opcodeSpans = find(opcodeFile, /NPC_CHANGETYPE(?:_KEEPALL)?/).map((span) => sources.add(span));

    const npcOpsFile = 'engine/script/handlers/NpcOps.ts';
    const npcOpsText = text(npcOpsFile);
    sourceRefs.type_change = [
        ...find(npcOpsFile, /ScriptOpcode\.NPC_CHANGETYPE\]/),
        ...find(npcOpsFile, /ScriptOpcode\.NPC_CHANGETYPE_KEEPALL\]/),
        ...find(npcOpsFile, /state\.activeNpc\.changeType\(npcType, duration(?:, false)?\)/),
        ...opcodeSpans.map((index) => sources.spans[index]),
    ].map((span) => sources.add(span));

    const npcFile = 'engine/entity/Npc.ts';
    const npcText = text(npcFile);
    const worldFile = 'engine/World.ts';
    const worldText = text(worldFile);
    sourceRefs.type_change.push(
        ...find(npcFile, /changeType\(type: number, duration: number/).map((span) => sources.add(span)),
        ...find(npcFile, /this\.type = type/).map((span) => sources.add(span)),
    );
    sourceRefs.reversion = [
        ...find(npcFile, /this\.baseType = type/),
        ...find(npcFile, /this\.resetOnRevert = reset/),
        ...find(npcFile, /this\.setLifeCycle\(duration\)/),
        ...find(npcFile, /this\.revertType\(\);/),
        ...find(npcFile, /private revertType\(\)/),
        ...find(npcFile, /this\.type = this\.baseType/),
        ...find(npcFile, /World\.removeNpc\(this, -1\)/),
        ...find(npcFile, /World\.addNpc\(this, -1, false\)/),
        ...find(worldFile, /npc\.resetEntity\(true\)/),
    ].map((span) => sources.add(span));

    const providerFile = 'engine/script/ScriptProvider.ts';
    const providerText = text(providerFile);
    const providerSpans = [
        ...find(providerFile, /type << 10/),
        ...find(providerFile, /category << 10/),
        ...find(providerFile, /return script;/),
        ...find(providerFile, /return ScriptProvider\.scriptLookup\.get\(trigger\)/),
    ].map((span) => sources.add(span));
    sourceRefs.numeric_dispatch.push(...providerSpans);

    const triggerFile = 'engine/script/ServerTriggerType.ts';
    const parsedTriggerCodes = parseTriggerCodes(text(triggerFile));
    const apSlots = [...parsedTriggerCodes.ap.keys()].sort((left, right) => left - right);
    const opSlots = [...parsedTriggerCodes.op.keys()].sort((left, right) => left - right);
    const contiguousSlots = (slots: number[]) => slots.length > 0 && slots.every((slot, index) => slot === index + 1);
    const contiguousCodes = (codes: ReadonlyMap<number, number>, slots: readonly number[]) => {
        const base = codes.get(1);
        return base !== undefined && slots.every((slot, index) => codes.get(slot) === base + index);
    };
    const matchingTriggerSlots = apSlots.length === opSlots.length && apSlots.every((slot, index) => slot === opSlots[index]);
    const apBase = parsedTriggerCodes.ap.get(1);
    const opBase = parsedTriggerCodes.op.get(1);
    const opTriggerOffset = apBase !== undefined && opBase === apBase + 7 && apSlots.every((slot) => parsedTriggerCodes.op.get(slot) === parsedTriggerCodes.ap.get(slot)! + 7);
    sourceRefs.numeric_dispatch.push(...find(triggerFile, /^\s*(?:AP|OP)NPC\d+\s*=/).map((span) => sources.add(span)));

    const unexpectedMutationSites: SourceSpan[] = [];
    for (const [logical, file] of files) {
        file.text.split(/\r?\n/).forEach((line, index) => {
            if (/\b[A-Za-z_$]*[Nn]pc[A-Za-z_$]*\.type\s*=(?!=)/.test(line)) {
                unexpectedMutationSites.push(lineSpan(file.relative, index + 1));
            }

            if (!/\bchangeType\s*\(/.test(line)) return;
            const knownDeclaration = logical === npcFile && /changeType\s*\(\s*type:\s*number\s*,\s*duration:\s*number/.test(line);
            const knownHandler = logical === npcOpsFile && /state\.activeNpc\.changeType\(npcType, duration(?:, false)?\)/.test(line);
            if (!knownDeclaration && !knownHandler) unexpectedMutationSites.push(lineSpan(file.relative, index + 1));
        });
    }

    const typeAssignments = npcText.match(/^[ \t]*this\.type\s*=\s*type\s*;/gm) ?? [];
    const baseTypeAssignments = npcText.match(/^[ \t]*this\.type\s*=\s*this\.baseType\s*;/gm) ?? [];
    const allNpcTypeAssignments = npcText.match(/^[ \t]*this\.type\s*=/gm) ?? [];
    const hasExpectedNpcTypeWrites = typeAssignments.length === 2 && baseTypeAssignments.length === 2 && allNpcTypeAssignments.length === 4;

    const opDecoderMatch = /code\s*>=\s*(\d+)\s*&&\s*code\s*<\s*(\d+)[\s\S]*?this\.op\[code\s*-\s*(\d+)\]\s*=\s*dat\.gjstr\(\)/.exec(npcTypeText);
    const opDecoder = opDecoderMatch !== null && Number(opDecoderMatch[1]) === Number(opDecoderMatch[3]);
    const optionSlotsFitDecoder = opDecoderMatch !== null && apSlots.every((slot) => slot <= Number(opDecoderMatch[2]) - Number(opDecoderMatch[1]));
    const triggerCodesComplete = parsedTriggerCodes.complete &&
        contiguousSlots(apSlots) &&
        contiguousSlots(opSlots) &&
        contiguousCodes(parsedTriggerCodes.ap, apSlots) &&
        contiguousCodes(parsedTriggerCodes.op, opSlots) &&
        matchingTriggerSlots &&
        opTriggerOffset &&
        optionSlotsFitDecoder;
    const dispatch = /NpcType\.get\(npc\.type\)/.test(opHandlerText) &&
        /npcType\.op\[message\.op\s*-\s*1\] === null/.test(opHandlerText) &&
        /npcType\.op\[message\.op\s*-\s*1\] === 'hidden'/.test(opHandlerText) &&
        /ServerTriggerType\.APNPC1\s*\+\s*\(message\.op\s*-\s*1\)/.test(opHandlerText);
    const opTriggerLookup = /getOpTrigger\(\)[\s\S]*?\n    \}/.exec(playerText)?.[0] ?? '';
    const apTriggerLookup = /getApTrigger\(\)[\s\S]*?\n    \}/.exec(playerText)?.[0] ?? '';
    const interactionDispatch = /tryInteract\(allowOpScenery: boolean\)[\s\S]*?\n    \}/.exec(playerText)?.[0] ?? '';
    const playerDispatch = /this\.targetOp \+ 7, typeId, categoryId/.test(opTriggerLookup) &&
        /this\.targetOp, typeId, categoryId/.test(apTriggerLookup) &&
        /const opTrigger = this\.getOpTrigger\(\);[\s\S]*?const apTrigger = this\.getApTrigger\(\);[\s\S]*?if \(opTrigger &&[\s\S]*?this\.inOperableDistance\(this\.target\)\)[\s\S]*?else if \(apTrigger && this\.inApproachDistance/.test(interactionDispatch);
    const providerLookup = /static getByTrigger\(trigger:[\s\S]*?\n    \}/.exec(providerText)?.[0] ?? '';
    const exactFallback = /scriptLookup\.get\(trigger \| \(0x2 << 8\) \| \(type << 10\)\)[\s\S]*?return script;[\s\S]*?scriptLookup\.get\(trigger \| \(0x1 << 8\) \| \(category << 10\)\)[\s\S]*?return script;[\s\S]*?return ScriptProvider\.scriptLookup\.get\(trigger\)/.test(providerLookup);
    const hasChanges = /\[ScriptOpcode\.NPC_CHANGETYPE\]/.test(npcOpsText) &&
        /\[ScriptOpcode\.NPC_CHANGETYPE_KEEPALL\]/.test(npcOpsText) &&
        /state\.activeNpc\.changeType\(npcType, duration\)/.test(npcOpsText) &&
        /\[ScriptOpcode\.NPC_CHANGETYPE_KEEPALL\]:[\s\S]*?state\.activeNpc\.changeType\(npcType, duration, false\)/.test(npcOpsText);
    const hasReversion = /this\.baseType = type/.test(npcText) &&
        /changeType\(type: number, duration: number/.test(npcText) &&
        /this\.setLifeCycle\(duration\)/.test(npcText) &&
        /this\.revertType\(\);/.test(npcText) &&
        /private revertType\(\)/.test(npcText) &&
        /World\.removeNpc\(this, -1\)/.test(npcText) &&
        /World\.addNpc\(this, -1, false\)/.test(npcText) &&
        /npc\.resetEntity\(true\)/.test(worldText) &&
        /this\.type = this\.baseType/.test(npcText) &&
        hasExpectedNpcTypeWrites;
    const hasTypeDecoderGuard = /Unrecognized npc config code/.test(npcTypeText);
    const requiredPresent = [...required].every((relative) => {
        const logical = relative.startsWith('src/') ? relative.slice('src/'.length) : relative;
        return files.has(logical);
    });
    const complete = requiredPresent && opDecoder && triggerCodesComplete && dispatch && playerDispatch && exactFallback && hasChanges && hasReversion && hasTypeDecoderGuard && unexpectedMutationSites.length === 0;
    sourceRefs.numeric_dispatch = uniqueRefs(sourceRefs.numeric_dispatch);
    sourceRefs.type_change = uniqueRefs(sourceRefs.type_change);
    sourceRefs.reversion = uniqueRefs(sourceRefs.reversion);
    sourceRefs.static_type_decoder = uniqueRefs(sourceRefs.static_type_decoder);
    return { complete, trigger_codes: parsedTriggerCodes, source_refs: sourceRefs, unexpected_mutation_sites: unexpectedMutationSites.map((span) => sources.add(span)) };
}

function normalizeScriptCode(raw: string) {
    const code = stripComment(raw).trim();
    let output = '';
    let quoted = false;
    let escaped = false;
    let pendingSpace = false;
    for (const ch of code) {
        if (quoted) {
            output += ch;
            if (escaped) escaped = false;
            else if (ch === '\\') escaped = true;
            else if (ch === '"') quoted = false;
            continue;
        }
        if (ch === '"') {
            if (pendingSpace && output) output += ' ';
            pendingSpace = false;
            quoted = true;
            output += ch;
        } else if (/\s/.test(ch)) {
            pendingSpace = true;
        } else {
            if (pendingSpace && output) output += ' ';
            pendingSpace = false;
            output += ch;
        }
    }
    return output;
}

type ScriptBlock = {
    kind: string;
    target: string | null;
    global: boolean;
    source: SourceSpan;
    operations: string[];
};

function parseScriptBlocks(file: string, text: string): ScriptBlock[] {
    return parseRs2Blocks(file, text).map((block) => {
        const target = block.name.trim();
        return {
            kind: block.kind.toLowerCase(),
            target,
            global: target === '_',
            source: block.span,
            operations: block.body.map((entry) => normalizeScriptCode(entry.text)).filter(Boolean),
        };
    });
}

function sourceCalls(text: string) {
    const lines = text.split(/\r?\n/).map((line) => stripComment(line));
    const joined = lines.join('\n');
    const lineStarts = [0];
    for (let i = 0; i < joined.length; i += 1) if (joined[i] === '\n') lineStarts.push(i + 1);
    const lineAt = (offset: number) => {
        let low = 0;
        let high = lineStarts.length;
        while (low + 1 < high) {
            const mid = (low + high) >>> 1;
            if (lineStarts[mid] <= offset) low = mid;
            else high = mid;
        }
        return low + 1;
    };
    const calls: { name: 'npc_changetype' | 'npc_changetype_keepall'; args: string[]; line: number; malformed: boolean }[] = [];
    let cursor = 0;
    let quoted = false;
    let escaped = false;
    while (cursor < joined.length) {
        const ch = joined[cursor];
        if (quoted) {
            if (escaped) escaped = false;
            else if (ch === '\\') escaped = true;
            else if (ch === '"') quoted = false;
            cursor += 1;
            continue;
        }
        if (ch === '"') {
            quoted = true;
            cursor += 1;
            continue;
        }
        if (!/[A-Za-z_]/.test(ch)) {
            cursor += 1;
            continue;
        }
        const start = cursor;
        cursor += 1;
        while (cursor < joined.length && /[A-Za-z0-9_]/.test(joined[cursor])) cursor += 1;
        const word = joined.slice(start, cursor).toLowerCase();
        if (word !== 'npc_changetype' && word !== 'npc_changetype_keepall') continue;
        let opening = cursor;
        while (opening < joined.length && /\s/.test(joined[opening])) opening += 1;
        if (joined[opening] !== '(') continue;
        const args: string[] = [];
        let depth = 0;
        let argStart = opening + 1;
        let inString = false;
        let escapedArg = false;
        let end = opening + 1;
        for (; end < joined.length; end += 1) {
            const token = joined[end];
            if (inString) {
                if (escapedArg) escapedArg = false;
                else if (token === '\\') escapedArg = true;
                else if (token === '"') inString = false;
                continue;
            }
            if (token === '"') inString = true;
            else if (token === '(') depth += 1;
            else if (token === ')') {
                if (depth === 0) {
                    args.push(joined.slice(argStart, end).trim());
                    break;
                }
                depth -= 1;
            } else if (token === ',' && depth === 0) {
                args.push(joined.slice(argStart, end).trim());
                argStart = end + 1;
            }
        }
        const malformed = end >= joined.length || joined[end] !== ')';
        calls.push({
            name: word as 'npc_changetype' | 'npc_changetype_keepall',
            args: malformed ? args : args.filter((argument, index) => index < 2),
            line: lineAt(start),
            malformed,
        });
        cursor = malformed ? joined.length : end + 1;
    }
    return calls;
}

type TriggerTarget = { kind: Exclude<NpcTransformTriggerKind, 'unresolved' | 'none'>; id: number | null; source: SourceSpan | null };
type ScriptTrigger = {
    code: number;
    target: TriggerTarget;
    source: SourceSpan;
    fingerprint: string;
};

function resolveTriggerTarget(target: string | null, global: boolean, npcs: StrictPack, categories: StrictPack): TriggerTarget | null {
    if ((global && target === null) || target === '_') return { kind: 'global', id: null, source: null };
    if (target === null) return null;
    const categoryName = target.startsWith('_') ? target.slice(1) : null;
    if (categoryName !== null) {
        const category = categories.byName.get(categoryName);
        return category ? { kind: 'category', id: category.id, source: category.source } : null;
    }
    const npc = npcs.byName.get(target);
    return npc ? { kind: 'type', id: npc.id, source: npc.source } : null;
}

function parseContextTypes(
    kind: string,
    target: string | null,
    global: boolean,
    triggerKinds: ReadonlySet<string>,
    npcs: StrictPack,
    categories: StrictPack,
    npcTypes: readonly NpcTypeFact[],
): { ids: number[] | null; reason: NpcTransformUnresolvedReason | null } {
    if (!triggerKinds.has(kind.toUpperCase())) return { ids: null, reason: 'unknown_transform_context' };
    if (!kind.toLowerCase().includes('npc') && !kind.toLowerCase().startsWith('ai_')) return { ids: null, reason: 'unknown_transform_context' };
    if (global && target === null) return { ids: null, reason: 'unknown_transform_context' };
    if (target === null) return { ids: null, reason: 'unknown_transform_context' };
    const resolved = resolveTriggerTarget(target, false, npcs, categories);
    if (!resolved) return { ids: null, reason: 'unknown_transform_context' };
    if (resolved.kind === 'type' && resolved.id !== null) return { ids: [resolved.id], reason: null };
    if (resolved.kind === 'category' && resolved.id !== null) {
        return { ids: npcTypes.filter((type) => type.category_id === resolved.id).map((type) => type.npc_type_id), reason: null };
    }
    return { ids: null, reason: 'unknown_transform_context' };
}

type TriggerSelection = {
    trigger: ScriptTrigger | null;
    reason: NpcTransformUnresolvedReason | null;
    evidence: ScriptTrigger[];
};

function selectTriggerForType(
    triggers: readonly ScriptTrigger[],
    code: number,
    type: NpcTypeFact,
): TriggerSelection {
    const sameCode = triggers.filter((trigger) => trigger.code === code);
    const exact = sameCode.filter((trigger) => trigger.target.kind === 'type' && trigger.target.id === type.npc_type_id);
    const category = type.category_id === null ? [] : sameCode.filter((trigger) => trigger.target.kind === 'category' && trigger.target.id === type.category_id);
    const global = sameCode.filter((trigger) => trigger.target.kind === 'global');
    const selected = exact.length > 0 ? exact : !type.category_known ? [] : category.length > 0 ? category : global;
    if (selected.length === 1) return { trigger: selected[0], reason: null, evidence: selected };
    if (selected.length > 1) return { trigger: null, reason: 'ambiguous_trigger', evidence: selected };
    if (!type.category_known && exact.length === 0) return { trigger: null, reason: 'unknown_category', evidence: [] };
    return { trigger: null, reason: null, evidence: [] };
}

function scriptOperationFingerprint(operations: readonly string[]) {
    return hashText(`npc-option-handler-v1\n${operations.join('\n')}`);
}

function actionFingerprint(op: ScriptTrigger | null, ap: ScriptTrigger | null) {
    return hashText(`npc-option-handler-pair-v1\n${JSON.stringify([op?.fingerprint ?? null, ap?.fingerprint ?? null])}`);
}

function addUniqueEdge(edges: NpcTransformEdge[], edge: NpcTransformEdge) {
    const refs = uniqueRefs(edge.source_refs);
    const normalized = { ...edge, source_refs: refs };
    const key = JSON.stringify(normalized);
    if (!edges.some((candidate) => JSON.stringify(candidate) === key)) edges.push(normalized);
}

function addUniqueUnresolved(rows: NpcTransformUnresolved[], row: NpcTransformUnresolved) {
    const normalized = { ...row, source_refs: uniqueRefs(row.source_refs) };
    const key = JSON.stringify(normalized);
    if (!rows.some((candidate) => JSON.stringify(candidate) === key)) rows.push(normalized);
}

function sortTypes(rows: NpcTransformNpcType[]) {
    rows.sort((a, b) => a.npc_type_id - b.npc_type_id);
}

function sortActions(rows: NpcTransformAction[]) {
    rows.sort((a, b) => a.npc_type_id - b.npc_type_id || a.slot - b.slot);
}

function sortEdges(rows: NpcTransformEdge[]) {
    rows.sort((a, b) => (a.from_npc_type_id ?? -1) - (b.from_npc_type_id ?? -1)
        || (a.to_npc_type_id ?? -1) - (b.to_npc_type_id ?? -1)
        || a.kind.localeCompare(b.kind)
        || a.change_mode.localeCompare(b.change_mode)
        || a.duration_expression.localeCompare(b.duration_expression)
        || a.source_refs.join(',').localeCompare(b.source_refs.join(',')));
}

function sortUnresolved(rows: NpcTransformUnresolved[]) {
    rows.sort((a, b) => (a.scope_npc_type_id ?? -1) - (b.scope_npc_type_id ?? -1)
        || (a.scope_slot ?? -1) - (b.scope_slot ?? -1)
        || a.reason.localeCompare(b.reason)
        || (a.expression ?? '').localeCompare(b.expression ?? '')
        || a.source_refs.join(',').localeCompare(b.source_refs.join(',')));
}

function sortProofs(rows: NpcTransformProof[]) {
    rows.sort((a, b) => a.npc_type_id - b.npc_type_id || a.slot - b.slot);
}

/** Prove one numeric slot over all source-discovered transitions and base-type reversions. */
export function proveNpcTransformSlot(input: NpcTransformProofInput): NpcTransformProof {
    const typeById = new Map(input.npc_types.map((type) => [type.npc_type_id, type]));
    const actionByPair = new Map(input.actions.map((action) => [`${action.npc_type_id}:${action.slot}`, action]));
    const reachable = new Set<number>();
    const evidence = new Set<number>(input.engine_source_refs.numeric_dispatch);
    const unresolvedRefs = new Set<number>();
    const baseQueue = [input.npc_type_id];
    const baseTypes = new Set<number>([input.npc_type_id]);
    const queue: number[] = [];
    const mismatched = new Set<number>();
    let uncertain = !input.engine_complete;

    const initial = actionByPair.get(`${input.npc_type_id}:${input.slot}`);
    for (const ref of initial?.source_refs ?? []) evidence.add(ref);
    if (!initial || initial.state !== 'present' || initial.operation_fingerprint === null || initial.trigger_code === null) uncertain = true;

    if (!input.engine_complete) {
        for (const refs of Object.values(input.engine_source_refs)) for (const ref of refs) evidence.add(ref);
    }
    while (baseQueue.length > 0) {
        const current = baseQueue.shift()!;
        for (const edge of input.transform_edges) {
            if (!edge.reverts_to_base_type || edge.to_npc_type_id !== current) continue;
            for (const ref of edge.source_refs) evidence.add(ref);
            if (edge.from_npc_type_id === null || !typeById.has(edge.from_npc_type_id)) {
                uncertain = true;
                continue;
            }
            if (!baseTypes.has(edge.from_npc_type_id)) {
                baseTypes.add(edge.from_npc_type_id);
                baseQueue.push(edge.from_npc_type_id);
            }
        }
    }
    queue.push(...baseTypes);
    for (const typeId of baseTypes) reachable.add(typeId);

    while (queue.length > 0) {
        const current = queue.shift()!;
        for (const edge of input.transform_edges) {
            if (edge.from_npc_type_id !== null && edge.from_npc_type_id !== current) continue;
            for (const ref of edge.source_refs) evidence.add(ref);
            if (edge.to_npc_type_id === null) {
                uncertain = true;
                continue;
            }
            if (!typeById.has(edge.to_npc_type_id)) {
                uncertain = true;
                continue;
            }
            if (!reachable.has(edge.to_npc_type_id)) {
                reachable.add(edge.to_npc_type_id);
                queue.push(edge.to_npc_type_id);
            }
        }
    }

    input.unresolved.forEach((row, index) => {
        const inScope = row.scope_npc_type_id === null || reachable.has(row.scope_npc_type_id);
        const slotInScope = row.scope_slot === null || row.scope_slot === input.slot;
        if (!inScope || !slotInScope) return;
        unresolvedRefs.add(index);
        for (const ref of row.source_refs) evidence.add(ref);
        uncertain = true;
    });

    const initialFingerprint = initial?.operation_fingerprint ?? null;
    for (const typeId of reachable) {
        const type = typeById.get(typeId);
        if (!type || type.config_state !== 'complete') {
            uncertain = true;
            if (type) for (const ref of type.source_refs) evidence.add(ref);
            continue;
        }
        const action = actionByPair.get(`${typeId}:${input.slot}`);
        if (!action) {
            if (typeId !== input.npc_type_id) {
                mismatched.add(typeId);
                for (const ref of type.source_refs) evidence.add(ref);
            }
            continue;
        }
        for (const ref of action.source_refs) evidence.add(ref);
        if (action.state !== 'present' || action.operation_fingerprint === null || action.trigger_code === null) {
            uncertain = true;
            continue;
        }
        if (initialFingerprint !== null && action.operation_fingerprint !== initialFingerprint) mismatched.add(typeId);
    }

    let status: NpcTransformStatus;
    if (uncertain) status = 'unavailable';
    else if (mismatched.size > 0) status = 'inconsistent';
    else if (reachable.size === 1) status = 'unchanged';
    else status = 'consistent';

    return {
        npc_type_id: input.npc_type_id,
        slot: input.slot,
        status,
        reachable_npc_type_ids: [...reachable].sort((a, b) => a - b),
        mismatched_npc_type_ids: [...mismatched].sort((a, b) => a - b),
        evidence_refs: [...evidence].sort((a, b) => a - b),
        unresolved_refs: [...unresolvedRefs].sort((a, b) => a - b),
    };
}

function scriptCalls(
    contentFiles: ReadonlyMap<string, string>,
    npcTypes: readonly NpcTypeFact[],
    npcPack: StrictPack,
    categoryPack: StrictPack,
    paramPack: StrictPack,
    triggerCodes: { ap: ReadonlyMap<number, number>; op: ReadonlyMap<number, number> },
    triggerKinds: ReadonlySet<string>,
    engine: NpcTransformClosureEvidence['engine'],
    sources: SourceRegistry,
) {
    const actions: NpcTransformAction[] = [];
    const unresolved: NpcTransformUnresolved[] = [];
    const blocksByFile = new Map<string, ScriptBlock[]>();
    const triggers: ScriptTrigger[] = [];
    const unknownTriggerRefsBySlot = new Map<number, number[]>();
    for (const [file, sourceText] of contentFiles) {
        const blocks = parseScriptBlocks(file, sourceText);
        blocksByFile.set(file, blocks);
        for (const block of blocks) {
            const match = /^(ap|op)npc(\d+)$/.exec(block.kind);
            if (!match) continue;
            const lane = match[1] === 'ap' ? 'ap' : 'op';
            const slot = Number(match[2]);
            const code = triggerCodes[lane].get(slot);
            const target = resolveTriggerTarget(block.target, block.global, npcPack, categoryPack);
            const sourceRef = sources.add(block.source);
            if (code === undefined || !target) {
                (unknownTriggerRefsBySlot.get(slot) ?? unknownTriggerRefsBySlot.set(slot, []).get(slot)!).push(sourceRef);
                continue;
            }
            triggers.push({ code, target, source: block.source, fingerprint: scriptOperationFingerprint(block.operations) });
        }
    }

    for (const type of npcTypes) {
        if (type.config_state !== 'complete' || !type.config) continue;
        for (const slot of [...type.config.operations.keys()].sort((left, right) => left - right)) {
            const optionRows = type.config.operations.get(slot)!;
            const activeRows = optionRows.filter((row) => row.value !== 'hidden');
            if (activeRows.length === 0) continue;

            const conflictingOptions = optionRows.length > 1 && !sameValues(optionRows);
            const opCode = triggerCodes.op.get(slot) ?? null;
            const apCode = triggerCodes.ap.get(slot) ?? null;
            const operationRefs = optionRows.map((row) => sources.add(row.source));
            const baseRefs = uniqueRefs([sources.add(type.pack_source), ...operationRefs]);
            if (conflictingOptions) {
                addUniqueUnresolved(unresolved, {
                    scope_npc_type_id: type.npc_type_id,
                    scope_slot: slot,
                    reason: 'ambiguous_npc_config',
                    expression: `op${slot}`,
                    source_refs: baseRefs,
                });
                actions.push({
                    npc_type_id: type.npc_type_id,
                    slot,
                    state: 'unknown',
                    label: null,
                    trigger_code: apCode,
                    trigger_kind: 'unresolved',
                    trigger_target_id: null,
                    operation_fingerprint: null,
                    source_refs: baseRefs,
                });
                continue;
            }

            let triggerKind: NpcTransformTriggerKind = 'none';
            let targetId: number | null = null;
            let state: NpcTransformActionState = 'present';
            let fingerprint: string | null = null;
            const unknownRefs = unknownTriggerRefsBySlot.get(slot) ?? [];
            if (opCode === null || apCode === null || unknownRefs.length > 0) {
                state = 'unknown';
                triggerKind = 'unresolved';
                // A slot header with an unresolvable subject keeps unknown_trigger_target:
                // the extractor cannot tell which NPC type the handler selects.
                addUniqueUnresolved(unresolved, {
                    scope_npc_type_id: type.npc_type_id,
                    scope_slot: slot,
                    reason: opCode === null || apCode === null ? 'engine_source_incomplete' : 'unknown_trigger_target',
                    expression: null,
                    source_refs: [...baseRefs, ...unknownRefs],
                });
            } else {
                const op = selectTriggerForType(triggers, opCode, type);
                const ap = selectTriggerForType(triggers, apCode, type);
                const selectionReason = op.reason ?? ap.reason;
                if (selectionReason !== null || (!op.trigger && !ap.trigger)) {
                    state = 'unknown';
                    triggerKind = selectionReason === null ? 'none' : 'unresolved';
                    // Every candidate subject resolves but no OP/AP handler selects this type,
                    // so the option has no handler to compare: missing_trigger_handler, not an
                    // unknown subject. Ambiguous or unknown-category selections keep their reason.
                    addUniqueUnresolved(unresolved, {
                        scope_npc_type_id: type.npc_type_id,
                        scope_slot: slot,
                        reason: selectionReason ?? 'missing_trigger_handler',
                        expression: null,
                        source_refs: [
                            ...baseRefs,
                            ...op.evidence.map((trigger) => sources.add(trigger.source)),
                            ...ap.evidence.map((trigger) => sources.add(trigger.source)),
                        ],
                    });
                } else {
                    const primary = op.trigger ?? ap.trigger!;
                    triggerKind = primary.target.kind;
                    targetId = primary.target.id;
                    fingerprint = actionFingerprint(op.trigger, ap.trigger);
                    for (const matched of [op.trigger, ap.trigger]) {
                        if (!matched) continue;
                        baseRefs.push(sources.add(matched.source));
                        if (matched.target.source !== null) baseRefs.push(sources.add(matched.target.source));
                    }
                }
            }
            actions.push({
                npc_type_id: type.npc_type_id,
                slot,
                state,
                label: optionRows[0].value,
                trigger_code: apCode,
                trigger_kind: triggerKind,
                trigger_target_id: targetId,
                operation_fingerprint: fingerprint,
                source_refs: uniqueRefs(baseRefs),
            });
        }
    }

    const edges: NpcTransformEdge[] = [];
    const typeById = new Map(npcTypes.map((type) => [type.npc_type_id, type]));
    for (const [file, sourceText] of contentFiles) {
        const blocks = blocksByFile.get(file) ?? [];
        for (const call of sourceCalls(sourceText)) {
            const callSource = lineSpan(file, call.line);
            const callRef = sources.add(callSource);
            const block = blocks.find((candidate) => candidate.source.first <= call.line && candidate.source.last >= call.line);
            const context = block
                ? parseContextTypes(block.kind, block.target, block.global, triggerKinds, npcPack, categoryPack, npcTypes)
                : { ids: null, reason: 'unknown_transform_context' as const };
            const fromIds = context.ids;
            if (context.reason) {
                addUniqueUnresolved(unresolved, {
                    scope_npc_type_id: null,
                    scope_slot: null,
                    reason: 'unknown_transform_context',
                    expression: block ? `${block.kind},${block.target ?? 'global'}` : 'script context not parsed',
                    source_refs: [callRef, ...(block ? [sources.add(block.source)] : [])],
                });
            }
            if (call.malformed || call.args.length < 2 || !call.args[0]) {
                addUniqueUnresolved(unresolved, {
                    scope_npc_type_id: fromIds?.length === 1 ? fromIds[0] : null,
                    scope_slot: null,
                    reason: 'unsupported_transform_expression',
                    expression: call.args[0] ?? null,
                    source_refs: [callRef],
                });
                addUniqueEdge(edges, {
                    from_npc_type_id: fromIds?.length === 1 ? fromIds[0] : null,
                    to_npc_type_id: null,
                    kind: call.args[0] && NPC_PARAM.test(call.args[0]) ? 'npc_param' : 'scripted',
                    change_mode: call.name === 'npc_changetype' ? 'reset' : 'keepall',
                    duration_expression: call.args[1] ?? '',
                    reverts_to_base_type: true,
                    source_refs: [callRef, ...engine.source_refs.type_change, ...engine.source_refs.reversion],
                });
                continue;
            }
            const targetExpression = call.args[0].trim();
            const durationExpression = call.args[1].trim();
            const paramMatch = NPC_PARAM.exec(targetExpression);
            const kind: NpcTransformEdgeKind = paramMatch ? 'npc_param' : 'scripted';
            const sourceIds = fromIds;
            const resolvedTargets: { from: number | null; to: number; refs: number[] }[] = [];
            const unresolvedScopes = new Map<number | null, NpcTransformUnresolvedReason>();
            const unresolvedSourceRefs: number[] = [];
            let dynamicReason: NpcTransformUnresolvedReason | null = null;
            const nextNpcTypeParam = paramMatch?.[1] === 'next_npc_type' ? paramPack.byName.get('next_npc_type') : undefined;
            if (nextNpcTypeParam) unresolvedSourceRefs.push(sources.add(nextNpcTypeParam.source));
            if (paramMatch && paramMatch[1] === 'next_npc_type') {
                if (!sourceIds?.length) {
                    dynamicReason = 'dynamic_transform_target';
                } else if (!nextNpcTypeParam) {
                    for (const sourceId of sourceIds) unresolvedScopes.set(sourceId, 'dynamic_transform_target');
                } else {
                    for (const sourceId of sourceIds) {
                        const sourceType = typeById.get(sourceId);
                        if (sourceType) {
                            unresolvedSourceRefs.push(sources.add(sourceType.pack_source));
                            if (sourceType.config) unresolvedSourceRefs.push(sources.add(sourceType.config.source));
                        }
                        const values = sourceType?.config?.params.get('next_npc_type') ?? [];
                        unresolvedSourceRefs.push(...values.map((value) => sources.add(value.source)));
                        if (!sourceType || sourceType.config_state !== 'complete' || values.length !== 1 || !values[0].value || values[0].value.toLowerCase() === 'null') {
                            const reason = sourceType?.config_state === 'missing' ? 'missing_npc_config' : 'dynamic_transform_target';
                            unresolvedScopes.set(sourceId, reason);
                            continue;
                        }
                        const target = npcPack.byName.get(values[0].value);
                        if (!target) {
                            unresolvedScopes.set(sourceId, 'dynamic_transform_target');
                            continue;
                        }
                        resolvedTargets.push({
                            from: sourceId,
                            to: target.id,
                            refs: [sources.add(values[0].source), sources.add(nextNpcTypeParam.source), sources.add(target.source)],
                        });
                    }
                }
            } else if (!paramMatch && INTEGER.test(targetExpression)) {
                const id = Number(targetExpression);
                const target = npcPack.byId.get(id);
                if (target && typeById.has(id)) {
                    for (const sourceId of sourceIds ?? [null]) resolvedTargets.push({ from: sourceId, to: id, refs: [sources.add(target.source)] });
                } else dynamicReason = 'dynamic_transform_target';
            } else if (!paramMatch && /^[A-Za-z][A-Za-z0-9_]*$/.test(targetExpression)) {
                const target = npcPack.byName.get(targetExpression);
                if (target && typeById.has(target.id)) {
                    for (const sourceId of sourceIds ?? [null]) resolvedTargets.push({ from: sourceId, to: target.id, refs: [sources.add(target.source)] });
                } else dynamicReason = 'dynamic_transform_target';
            } else {
                dynamicReason = paramMatch ? 'unsupported_transform_expression' : 'dynamic_transform_target';
            }
            if (paramMatch && paramMatch[1] !== 'next_npc_type') dynamicReason = 'unsupported_transform_expression';
            if (dynamicReason !== null) {
                for (const sourceId of sourceIds ?? [null]) unresolvedScopes.set(sourceId, dynamicReason);
            }
            for (const target of resolvedTargets) {
                addUniqueEdge(edges, {
                    from_npc_type_id: target.from,
                    to_npc_type_id: target.to,
                    kind,
                    change_mode: call.name === 'npc_changetype' ? 'reset' : 'keepall',
                    duration_expression: durationExpression,
                    reverts_to_base_type: true,
                    source_refs: [callRef, ...target.refs, ...engine.source_refs.type_change, ...engine.source_refs.reversion],
                });
            }
            if (unresolvedScopes.size === 0 && resolvedTargets.length === 0) {
                for (const sourceId of sourceIds ?? [null]) unresolvedScopes.set(sourceId, 'dynamic_transform_target');
            }
            for (const [scope, reason] of unresolvedScopes) {
                addUniqueEdge(edges, {
                    from_npc_type_id: scope,
                    to_npc_type_id: null,
                    kind,
                    change_mode: call.name === 'npc_changetype' ? 'reset' : 'keepall',
                    duration_expression: durationExpression,
                    reverts_to_base_type: true,
                    source_refs: [callRef, ...unresolvedSourceRefs, ...engine.source_refs.type_change, ...engine.source_refs.reversion],
                });
                addUniqueUnresolved(unresolved, {
                    scope_npc_type_id: scope,
                    scope_slot: null,
                    reason,
                    expression: targetExpression,
                    source_refs: [callRef, ...unresolvedSourceRefs, ...engine.source_refs.type_change, ...engine.source_refs.reversion],
                });
            }
        }
    }
    return { actions, edges, unresolved };
}



/**
 * Build the complete source-derived NPC option and transform closure for one selected content pack.
 * `engineRoot` may be the engine repository root or its `src` directory.
 */
export function extractNpcTransformClosure(engineRoot: string, contentRoot: string): NpcTransformClosureEvidence {
    const resolvedEngine = normalizeEngineRoot(engineRoot);
    const paths = npcTransformInputPaths(engineRoot, contentRoot);
    const contentPaths = paths.content;
    const engineRead = readInputs(resolvedEngine.root, paths.engine);
    const contentRead = readInputs(contentRoot, contentPaths);
    const engineInputs = engineRead.hashes;
    const contentInputs = contentRead.hashes;
    const sources = new SourceRegistry();

    const npcPackText = contentRead.texts.get('pack/npc.pack');
    const categoryPackText = contentRead.texts.get('pack/category.pack');
    const scriptPackText = contentRead.texts.get('pack/script.pack');
    const paramPackText = contentRead.texts.get('pack/param.pack');
    if (!npcPackText || !categoryPackText || !scriptPackText || !paramPackText) throw new Error('npc_transforms: selected pack inputs are incomplete');
    const npcPack = parseStrictPack('pack/npc.pack', npcPackText);
    const categoryPack = parseStrictPack('pack/category.pack', categoryPackText);
    const paramPack = parseStrictPack('pack/param.pack', paramPackText);
    // The source script table participates in the selected content identity; NPC aliases come from npc.pack.
    if (parsePack(scriptPackText).size === 0) throw new Error('npc_transforms: selected script pack is empty');

    const engineFileMap = engineRead.texts;
    const engine = engineSemantics(engineFileMap, sources);
    const engineText = (relative: string) => engineFileMap.get(`src/${relative}`) ?? engineFileMap.get(relative);
    const triggerText = engineText('engine/script/ServerTriggerType.ts');
    const triggerCodes = engine.trigger_codes;
    const triggerKinds = new Set<string>();
    if (triggerText) {
        triggerText.split(/\r?\n/).forEach((line) => {
            const match = /^\s*([A-Z][A-Z0-9_]*)\s*=\s*\d+\s*,?\s*$/.exec(stripComment(line));
            if (match) triggerKinds.add(match[1]);
        });
    }
    const allNpcConfigs: NpcConfigFact[] = [];
    for (const file of contentPaths.filter((name) => name.endsWith('.npc'))) {
        const text = contentRead.texts.get(file);
        if (text === undefined) continue;
        allNpcConfigs.push(...parseNpcConfigFile(file, text));
    }
    const configsByName = new Map<string, NpcConfigFact[]>();
    for (const config of allNpcConfigs) (configsByName.get(config.name) ?? configsByName.set(config.name, []).get(config.name)!).push(config);

    const typeFacts: NpcTypeFact[] = [];
    const npcTypes: NpcTransformNpcType[] = [];
    const unresolved: NpcTransformUnresolved[] = [];
    for (const entry of npcPack.entries) {
        const configMatches = configsByName.get(entry.name) ?? [];
        let configState: NpcTransformTypeState = configMatches.length === 0 ? 'missing' : configMatches.length === 1 ? 'complete' : 'ambiguous';
        const config = configState === 'complete' ? configMatches[0] : null;
        let categoryId: number | null = null;
        const typeSourceRefs = [sources.add(entry.source)];
        if (configMatches.length > 0) typeSourceRefs.push(...configMatches.map((row) => sources.add(row.source)));
        if (configState !== 'complete') {
            addUniqueUnresolved(unresolved, {
                scope_npc_type_id: entry.id,
                scope_slot: null,
                reason: configState === 'missing' ? 'missing_npc_config' : 'ambiguous_npc_config',
                expression: entry.name,
                source_refs: typeSourceRefs,
            });
        }
        if (config) {
            const categoryRows = config.categories;
            if (categoryRows.length === 0) {
                categoryId = null;
                typeSourceRefs.push(...engine.source_refs.static_type_decoder);
            } else if (sameValues(categoryRows)) {
                typeSourceRefs.push(...categoryRows.map((row) => sources.add(row.source)));
                const category = categoryPack.byName.get(categoryRows[0].value);
                if (category) {
                    categoryId = category.id;
                    typeSourceRefs.push(sources.add(category.source));
                } else {
                    categoryId = null;
                    addUniqueUnresolved(unresolved, {
                        scope_npc_type_id: entry.id,
                        scope_slot: null,
                        reason: 'unknown_category',
                        expression: categoryRows[0].value,
                        source_refs: [sources.add(categoryRows[0].source), ...typeSourceRefs],
                    });
                }
            } else {
                categoryId = null;
                addUniqueUnresolved(unresolved, {
                    scope_npc_type_id: entry.id,
                    scope_slot: null,
                    reason: 'ambiguous_npc_config',
                    expression: 'conflicting category fields',
                    source_refs: [...categoryRows.map((row) => sources.add(row.source)), ...typeSourceRefs],
                });
            }
        }
        const fact: NpcTypeFact = { npc_type_id: entry.id, name: entry.name, pack_source: entry.source, config_state: configState, config, category_id: categoryId, category_known: config !== null && (config.categories.length === 0 || categoryId !== null) };
        typeFacts.push(fact);
        npcTypes.push({ npc_type_id: entry.id, category_id: categoryId, config_state: configState, source_refs: uniqueRefs(typeSourceRefs) });
    }
    sortTypes(npcTypes);

    const scriptInputMap = new Map(contentPaths.filter((file) => file.endsWith('.rs2')).map((file) => [file, contentRead.texts.get(file)!]));
    const scans = scriptCalls(
        scriptInputMap,
        typeFacts,
        npcPack,
        categoryPack,
        paramPack,
        triggerCodes,
        triggerKinds,
        { complete: engine.complete, source_refs: engine.source_refs },
        sources,
    );
    const actions = scans.actions;
    unresolved.push(...scans.unresolved);

    sortActions(actions);
    const transformEdges = scans.edges;
    for (const row of unresolved) row.source_refs = uniqueRefs(row.source_refs);
    sortEdges(transformEdges);
    sortUnresolved(unresolved);

    const engineComplete = engine.complete;
    if (!engineComplete) {
        const missingRefs = [...engine.source_refs.numeric_dispatch, ...engine.source_refs.type_change, ...engine.source_refs.reversion, ...engine.source_refs.static_type_decoder, ...engine.unexpected_mutation_sites];
        addUniqueUnresolved(unresolved, {
            scope_npc_type_id: null,
            scope_slot: null,
            reason: 'engine_source_incomplete',
            expression: 'required NPC dispatch, transform, reversion or type decoder semantics did not match',
            source_refs: missingRefs,
        });
        sortUnresolved(unresolved);
    }

    const proofs = actions.map((action) => proveNpcTransformSlot({
        npc_type_id: action.npc_type_id,
        slot: action.slot,
        npc_types: npcTypes,
        actions,
        transform_edges: transformEdges,
        unresolved,
        engine_complete: engineComplete,
        engine_source_refs: engine.source_refs,
    }));
    sortProofs(proofs);

    return {
        schema: 1,
        source_inputs: { engine: engineInputs, content: contentInputs },
        source_spans: sources.spans,
        npc_types: npcTypes,
        actions,
        transform_edges: transformEdges,
        unresolved,
        proofs,
        engine: {
            complete: engineComplete,
            source_refs: engine.source_refs,
        },
    };
}
