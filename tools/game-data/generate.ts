import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { parseRows, parsePack, integer, parseParamDefinitions, parseMapsquarePath, jm2SectionName, parseJm2LocPlacements, worldFromMapsquare, requireGatherText, placementMapFiles } from './extractors/common.ts';
import { extractGatheringFamily, compareCodepoint, gatherResources, gatherSites, hiddenSitesBytes, hiddenSitesRecord, HIDDEN_SITES_FILE, miningHazards, type GatherSiteWire, type HiddenSitesRecord, type MethodWire } from './extractors/gathering.ts';
import { extractQuestIdentityFacts } from './extractors/quests.ts';
import { assertPrereqStepFacts, extractPrereqStepsFacts, PREREQ_STEPS_SCHEMA } from './extractors/prereq-steps.ts';
import { extractQuestStartFacts } from './extractors/quest-starts.ts';
import { extractCombatStyleFacts, parseCombatScripts, buildSpellMaxHits, extractSpecialWeaponFacts, extractItemCombatBonus, extractMeleeWeaponTiers, type MeleeWeaponTierFacts } from './extractors/combat.ts';
import { extractDuelRuleButtons, extractDuelZones, type DuelRuleButtonWire } from './extractors/duel.ts';
import { consumptionRule, parseConsumeMessageDelays, parseConsumptionEffects } from './extractors/consumption.ts';
import { extractNpcNamesFacts } from './extractors/npc-names.ts';
import { extractLocNamesFacts } from './extractors/loc-names.ts';
import { extractNpcPlacementsFacts } from './extractors/npc-placements.ts';
import { extractKaramjaFacts } from './extractors/karamja.ts';
import { extractDebugCatalog, engineHandlerRelative } from './extractors/debug.ts';
import { extractDialogueUiFacts } from './extractors/dialogue-ui.ts';
import { loadServerOrderedFacts } from './extractors/server-ordered.ts';
import { extractNpcTransformClosure, type NpcTransformClosureEvidence } from './extractors/npc-transforms.ts';

type ObjType = { id: number; debugname: string | null; name: string | null; cost: number; stackable: boolean; members: boolean; certlink: number; certtemplate: number; wearpos: number; wearpos2: number; wearpos3: number; manwear: number; tradeable?: boolean; countobj?: ArrayLike<number> | null; category?: number; params?: Map<number, number | string> };
type NpcType = { id: number; debugname?: string | null; name: string | null; respawnrate?: number };
/** `NpcType.respawnrate` default: "default to 1-minute" (`engine/src/cache/config/NpcType.ts:105`). */
const ENGINE_DEFAULT_NPC_RESPAWN_TICKS = 100;
export type SelectedItemRow = {
    alias: string | null; id: number; name: string | null; cost: number; stackable: boolean; members: boolean;
    certificate_link: number; certificate_template: number; wear_position: number; wear_position_2: number; wear_position_3: number;
    worn_model: boolean; category: string | null; tradeable: boolean; stack_variant: boolean; destroy_death?: boolean;
};

const root = path.resolve(import.meta.dirname, '../..');
type Revision = { revision: number; engine: string; content: string; output: string };
/** Every external source root is explicit: an unset variable fails closed and names itself. */
export function requireEnvPath(name: string, label: string): string {
    const value = process.env[name];
    if (!value) throw new Error(`set ${name} to ${label}`);
    return path.resolve(value);
}
export const revisions: Revision[] = [
    { revision: 274, engine: requireEnvPath('GAME_DATA_274_ENGINE', 'the 274 engine root'), content: requireEnvPath('GAME_DATA_274_CONTENT', 'the 274 content root'), output: path.join(root, 'crates/api/data/game-data/274.json') },
    { revision: 289, engine: requireEnvPath('GAME_DATA_289_ENGINE', 'the 289 engine root'), content: requireEnvPath('GAME_DATA_289_CONTENT', 'the 289 content root'), output: path.join(root, 'crates/api/data/game-data/289.json') },
    { revision: 377, get engine() { return requireEnvPath('GAME_DATA_377_ENGINE', 'the 377 engine root'); }, get content() { return requireEnvPath('GAME_DATA_377_CONTENT', 'the 377 content root'); }, output: path.join(root, 'crates/api/data/game-data/377.json') }
];
const dropContentFiles = [
    'scripts/_unpack/225/all.npc',
    'scripts/areas/area_kalphite/configs/kalphite.npc',
    'scripts/drop tables/scripts/giant.rs2',
    'scripts/drop tables/scripts/moss_giant.rs2',
    'scripts/drop tables/scripts/fire_giant.rs2',
    'scripts/drop tables/scripts/green_dragon.rs2',
    'scripts/drop tables/scripts/kalphite_queen.rs2',
    'scripts/drop tables/scripts/shared_droptables.rs2',
];
function dropContentFilesForRevision(revision: number): string[] {
    return revision === 377 ? [
        'scripts/_unpack/377/all.npc',
        'scripts/areas/area_kalphite/configs/kalphite.npc',
        'scripts/npc/configs/dragons.npc',
        ...dropContentFiles.filter(file => file.endsWith('.rs2'))
            .map(file => file.replace('scripts/drop tables/', 'scripts/drop_tables/')),
    ] : dropContentFiles;
}
const unpacked377HerbsFile = 'scripts/_unpack/377/all.obj';
const thievingLevelOnlySources = [
    { file: 'scripts/quests/quest_itexam/scripts/digsite_workman.rs2', aliases: ['digworkman1', 'digworkman2'] },
    { file: 'scripts/quests/quest_troll/scripts/troll_stronghold_camp_guard.rs2', aliases: ['troll_prison_guard1', 'troll_prison_guard2'] },
] as const;
export type FamilyDraft = { schema: number; payload: unknown };
export function familyBytes(draft: FamilyDraft, name = 'family') {
    if (!Number.isInteger(draft.schema) || draft.schema < 1 || draft.schema > 65535) throw new Error(`${name}: invalid extractor schema`);
    return `${JSON.stringify({ schema: draft.schema, facts: draft.payload }, null, 2)}\n`;
}
/** Static quest identity and observation metadata; no per-account progress. */
export const QUESTS_SCHEMA = 2;
export function extractQuestsFamily(content: string, revision: number): FamilyDraft {
    return { schema: QUESTS_SCHEMA, payload: extractQuestIdentityFacts(content, revision) };
}
/**
 * Item category ids are decoded from the selected cache and named by the
 * selected content's `pack/category.pack`; `-1` is the engine's unassigned value.
 * `tradeable` is the engine's own decode after load: opcode 15 (`tradeable=no`), a nonzero `dummyitem`, or a note of an
 * untradeable item. `stack_variant` marks the objs another obj names as a pile-size model (`countobj`); they repeat the
 * base name and are not separate items.
 */
export function categoryName(obj: ObjType, categories: ReadonlyMap<number, string>): string | null {
    const id = obj.category ?? -1;
    if (id === -1) return null;
    const name = categories.get(id);
    if (name === undefined) throw new Error(`obj ${obj.id}: category ${id} has no content name`);
    return name;
}

function row(obj: ObjType, piles: ReadonlySet<number>, categories: ReadonlyMap<number, string>, destroyDeathParam: number | undefined, pvpV0: boolean): SelectedItemRow {
    if (typeof obj.tradeable !== 'boolean') throw new Error(`obj ${obj.id}: engine decode has no tradeable flag`);
    // PvP V0 `GameItem.destroy_death`: true when the obj carries
    // `param=destroy_death,^true` (`player/configs/death.param:1-3`), through
    // the same param reader the combat extractor uses (combat.ts
    // `parameterValue`). A pre-V0 revision omits the field and the Rust field
    // defaults it.
    const destroyDeathValue = destroyDeathParam !== undefined ? obj.params?.get(destroyDeathParam) : undefined;
    const destroyDeath = destroyDeathValue === 1 || destroyDeathValue === '1';
    return {
        alias: obj.debugname,
        id: obj.id,
        name: obj.name,
        cost: obj.cost,
        stackable: obj.stackable,
        members: obj.members,
        certificate_link: obj.certlink,
        certificate_template: obj.certtemplate,
        wear_position: obj.wearpos,
        wear_position_2: obj.wearpos2,
        wear_position_3: obj.wearpos3,
        worn_model: obj.manwear !== -1,
        category: categoryName(obj, categories),
        tradeable: obj.tradeable,
        stack_variant: piles.has(obj.id),
        ...(pvpV0 ? { destroy_death: destroyDeath } : {}),
    };
}
function pileModels(objs: readonly ObjType[]) { const piles = new Set<number>(); for (const obj of objs) for (const pile of Array.from(obj.countobj ?? [])) if (pile > 0) piles.add(pile); return piles; }
function required(values: Record<string, string[][]>, key: string, rowName: string) { const value = values[key]?.[0]?.[0]; if (value === undefined) throw new Error(`${rowName}: missing ${key}`); return value; }
function boolean(values: Record<string, string[][]>, key: string, rowName: string) {
    const value = required(values, key, rowName);
    if (value !== 'true' && value !== 'false') throw new Error(`${rowName}: invalid ${key} ${value}`);
    return value === 'true';
}
function namedItem(itemIds: Map<string, { id: number; name: string | null }>, alias: string, label: string) {
    const item = itemIds.get(alias);
    if (!item) throw new Error(`${label}: unknown item ${alias}`);
    if (!item.name) throw new Error(`${label}: ${alias} has no display name`);
    return { alias, id: item.id, name: item.name };
}
function runeCosts(values: Record<string, string[][]>, rowName: string, itemIds: Map<string, { id: number; name: string | null }>) {
    const raw = values.runesrequired?.[0] ?? [];
    const runes: { alias: string; id: number; name: string; count: number }[] = [];
    for (let i = 0; i + 1 < raw.length; i += 2) {
        const alias = raw[i];
        if (!alias || alias === 'null') continue;
        const item = namedItem(itemIds, alias, `${rowName} rune`);
        runes.push({ ...item, count: integer(raw[i + 1], `${rowName} ${alias}`) });
    }
    return runes;
}
/** staff_spells autocast chooser: spell enum -> ssb index from auto_cast.rs2 button triggers. */
export function parseAutocastChooser(text: string) {
    const chooser = new Map<string, number>();
    const bySsb = new Map<number, string>();
    for (const raw of text.split(/\r?\n/)) {
        const line = raw.trim();
        const match = /^\[if_button,staff_spells:ssb(\d+)\] @set_autocast_spell\(\^([A-Za-z0-9_]+)\);$/.exec(line);
        if (!match) continue;
        const ssb = Number(match[1]);
        const spell = match[2];
        if (chooser.has(spell)) throw new Error(`autocast: duplicate chooser spell ^${spell}`);
        if (bySsb.has(ssb)) throw new Error(`autocast: duplicate chooser ssb${ssb}`);
        chooser.set(spell, ssb);
        bySsb.set(ssb, spell);
    }
    if (chooser.size === 0) throw new Error('autocast: no staff_spells chooser triggers');
    return chooser;
}
/** magic.if section -> manual cast `action=` label (Crumble undead, Iban blast, god strikes). */
export function parseMagicActions(text: string) {
    const out = new Map<string, string>();
    let current: string | null = null;
    for (const raw of text.split(/\r?\n/)) {
        const line = raw.trim();
        if (line.startsWith('[') && line.endsWith(']')) {
            current = line.slice(1, -1);
            continue;
        }
        if (!current || !line.startsWith('action=')) continue;
        const action = line.slice('action='.length);
        if (!action) throw new Error(`magic.if: empty action in ${current}`);
        if (out.has(current)) throw new Error(`magic.if: duplicate action section ${current}`);
        out.set(current, action);
    }
    return out;
}
/** One pair of dbrow string quotes around a selected message; anything else fails closed. */
function unquoteDbString(value: string, label: string) {
    if (value.length >= 2 && value.startsWith('"') && value.endsWith('"')) return value.slice(1, -1);
    throw new Error(`${label}: expected quoted string, got ${value}`);
}
export function extractMagicFacts(content: string, items: ObjType[]) {
    const itemIds = new Map(items.filter((item) => item.debugname !== null).map((item) => [item.debugname as string, { id: item.id, name: item.name }]));
    const interfaces = parsePack(fs.readFileSync(path.join(content, 'pack/interface.pack'), 'utf8'));
    const chooser = parseAutocastChooser(fs.readFileSync(path.join(content, 'scripts/skill_combat/scripts/player/auto_cast.rs2'), 'utf8'));
    const actions = parseMagicActions(fs.readFileSync(path.join(content, 'scripts/skill_magic/interfaces/magic.if'), 'utf8'));
    const spotanims = parsePack(fs.readFileSync(path.join(content, 'pack/spotanim.pack'), 'utf8'));
    const seenSpell = new Set<string>();
    const seenName = new Set<string>();
    const spells = [];
    for (const parsed of parseRows(fs.readFileSync(path.join(content, 'scripts/skill_combat/configs/magic/magic_combat_spells.dbrow'), 'utf8'))) {
        const spellcom = parsed.values.spellcom?.[0]?.[0];
        if (!spellcom) continue;
        if (parsed.values.continue_by_autocast?.[0]?.[0] !== 'true') continue;
        const spellField = required(parsed.values, 'spell', parsed.name);
        const spell = spellField.startsWith('^') ? spellField.slice(1) : spellField;
        if (!spell) throw new Error(`${parsed.name}: empty spell`);
        if (seenSpell.has(spell)) throw new Error(`${parsed.name}: duplicate spell ^${spell}`);
        seenSpell.add(spell);
        const action = actions.get(spell);
        if (!action) throw new Error(`${parsed.name}: missing magic.if action for ^${spell}`);
        const dbName = parsed.values.name?.[0]?.[0];
        if (dbName && dbName.toLowerCase() !== action.toLowerCase()) throw new Error(`${parsed.name}: name ${dbName} disagrees with magic.if action ${action}`);
        const name = dbName ?? action;
        if (!name) throw new Error(`${parsed.name}: missing spell name`);
        const folded = name.toLowerCase();
        if (seenName.has(folded)) throw new Error(`${parsed.name}: duplicate spell name ${name}`);
        seenName.add(folded);
        const componentId = interfaces.get(spellcom);
        if (componentId === undefined) throw new Error(`${parsed.name}: missing interface.pack entry ${spellcom}`);
        const ssb = chooser.get(spell);
        const wornrequired = parsed.values.wornrequired?.[0]?.[0];
        if (wornrequired !== undefined) namedItem(itemIds, wornrequired, `${parsed.name} required weapon`);
        const wornRaw = parsed.values.worn_reqmessage?.[0]?.[0];
        if (wornrequired !== undefined && wornRaw === undefined) throw new Error(`${parsed.name}: wornrequired without worn_reqmessage`);
        // Successful target gfx only. The shared miss splash is published separately
        // as failed_spell_impact; it and unresolvable aliases remain -1 per spell.
        const targetRaw = parsed.values.spotanim_target?.[0]?.[0];
        const targetAlias = targetRaw === undefined ? null : (targetRaw.startsWith('^') ? targetRaw.slice(1) : targetRaw);
        const impactSpotanim = targetAlias === null || targetAlias === 'failedspell_impact' ? -1 : (spotanims.get(targetAlias) ?? -1);
        spells.push({
            name,
            source_row: parsed.name,
            ssb: ssb ?? -1,
            component_id: componentId,
            autocast_selectable: ssb !== undefined,
            spellcom,
            level: integer(required(parsed.values, 'levelrequired', parsed.name), parsed.name),
            maxhit: integer(required(parsed.values, 'maxhit', parsed.name), parsed.name),
            members: boolean(parsed.values, 'members', parsed.name),
            wornrequired: wornrequired ?? null,
            worn_reqmessage: wornRaw === undefined ? null : unquoteDbString(wornRaw, parsed.name),
            impact_spotanim: impactSpotanim,
            continue_by_autocast: true,
            runes: runeCosts(parsed.values, parsed.name, itemIds),
        });
    }
    for (const [spell, ssb] of chooser) {
        if (!seenSpell.has(spell)) throw new Error(`autocast: chooser ^${spell} (ssb${ssb}) has no combat row`);
    }
    const byStaff = new Map<string, { alias: string; id: number; name: string; runes: Map<string, { alias: string; id: number; name: string }> }>();
    for (const parsed of parseRows(fs.readFileSync(path.join(content, 'scripts/skill_magic/configs/magic_staff.dbrow'), 'utf8'))) {
        const rune = namedItem(itemIds, required(parsed.values, 'rune', parsed.name), `${parsed.name} rune`);
        for (const row of parsed.values.staff ?? []) {
            const staff = namedItem(itemIds, row[0], `${parsed.name} staff`);
            const current = byStaff.get(staff.alias) ?? { ...staff, runes: new Map() };
            current.runes.set(rune.alias, rune);
            byStaff.set(staff.alias, current);
        }
    }
    const staves = [...byStaff.values()]
        .map((staff) => ({ alias: staff.alias, id: staff.id, name: staff.name, runes: [...staff.runes.values()] }))
        .sort((a, b) => a.name.localeCompare(b.name));
    // Shared miss splash as one selected scalar, never duplicated per spell.
    const failedSpellImpact = spotanims.get('failedspell_impact') ?? null;
    return { spells, staves, failed_spell_impact: failedSpellImpact };
}

const EQUIPMENT_FAMILY_ORDER = ['bows', 'crossbows', 'darts', 'arrows', 'bolts', 'melee_weapons', 'staffs'] as const;
type EquipmentFamilyId = (typeof EQUIPMENT_FAMILY_ORDER)[number];
const EQUIPMENT_CURATED_RELATIVE = 'tools/game-data/equipment-names.curated.json';

export type EquipmentNameEntry = {
    requested_name: string;
    disposition: 'resolved' | 'absent' | 'ambiguous';
    selected_name?: string;
    alias?: string;
    id?: number;
    wear_position?: number;
    disambiguation?: string;
    absent_class?: 'no_exact_selected_match' | 'exact_match_ineligible';
    candidates?: { alias: string; id: number; name: string; wear_position: number; note: string }[];
};

export type EquipmentExactNameJoin = {
    matching: 'exact_display_name_only';
    no_exact_selected_match_means: string;
    bolts: {
        generic_display_name: 'Bolts';
        generic_is_substitute: false;
        note: string;
    };
};

export type EquipmentNamesFacts = {
    exact_name_join: EquipmentExactNameJoin;
    bows: EquipmentNameEntry[];
    crossbows: EquipmentNameEntry[];
    darts: EquipmentNameEntry[];
    arrows: EquipmentNameEntry[];
    bolts: EquipmentNameEntry[];
    melee_weapons: EquipmentNameEntry[];
    staffs: EquipmentNameEntry[];
};

type EquipmentCurated = {
    schema: string;
    families: Record<EquipmentFamilyId, string[]>;
};

const EQUIPMENT_EXACT_NAME_JOIN: EquipmentExactNameJoin = {
    matching: 'exact_display_name_only',
    no_exact_selected_match_means: 'No exact display name in the selected item table. This is not a claim about all game content or global revision unavailability.',
    bolts: {
        generic_display_name: 'Bolts',
        generic_is_substitute: false,
        note: 'Frozen BOLTS members (Bronze bolts … Bone bolts) are absent under exact join. Generic Bolts / Bolts(p) are not substitutes and are not mapped onto those tier strings.',
    },
};

function isWearableRow(item: SelectedItemRow) {
    return item.wear_position >= 0 || item.wear_position_2 >= 0 || item.wear_position_3 >= 0;
}

function isBankNoteRow(item: SelectedItemRow) {
    return item.certificate_template >= 0;
}


export function loadEquipmentNamesCurated(repoRoot: string = root): EquipmentCurated {
    const curatedPath = path.join(repoRoot, EQUIPMENT_CURATED_RELATIVE);
    if (!fs.existsSync(curatedPath)) throw new Error(`equipment names curated missing at ${curatedPath}`);
    const curated = JSON.parse(fs.readFileSync(curatedPath, 'utf8')) as EquipmentCurated;
    if (curated.schema !== 'r018-equipment-names-curated-1') {
        throw new Error(`equipment names curated schema mismatch: ${curated.schema}`);
    }
    for (const family of EQUIPMENT_FAMILY_ORDER) {
        const names = curated.families[family];
        if (!Array.isArray(names) || names.length === 0) {
            throw new Error(`equipment names curated: missing family ${family}`);
        }
        const seen = new Set<string>();
        for (const name of names) {
            if (!name || typeof name !== 'string') throw new Error(`equipment names curated: malformed name in ${family}`);
            if (seen.has(name)) throw new Error(`equipment names curated: duplicate ${family} name ${name}`);
            seen.add(name);
        }
    }
    return curated;
}

function requireObjPack(objPack: Map<string, number> | undefined): Map<string, number> {
    if (!(objPack instanceof Map)) throw new Error('equipment names: missing required obj.pack');
    return objPack;
}

export function joinEquipmentName(
    family: EquipmentFamilyId,
    requestedName: string,
    items: SelectedItemRow[],
    objPack: Map<string, number>,
): EquipmentNameEntry {
    const pack = requireObjPack(objPack);
    const exact = items.filter((item) => item.name === requestedName);
    let pool = exact.filter((item) => !isBankNoteRow(item));
    const notes = exact.filter((item) => isBankNoteRow(item));
    if (family === 'bows' || family === 'crossbows' || family === 'melee_weapons' || family === 'staffs') {
        pool = pool.filter((item) => isWearableRow(item));
        if (family === 'bows') {
            pool = pool.filter((item) => item.alias !== null && !item.alias.startsWith('unstrung_'));
        }
    } else if (family === 'darts' || family === 'arrows' || family === 'bolts') {
        pool = pool.filter((item) => isWearableRow(item));
    }
    let blackDaggerPreference = false;
    if (family === 'melee_weapons' && requestedName === 'Black dagger' && pool.length > 1) {
        const standard = pool.filter((item) => item.alias === 'black_dagger');
        const quest = pool.filter((item) => item.alias === 'deathdagger');
        if (
            standard.length === 1
            && quest.length === 1
            && pack.get('black_dagger') === standard[0]!.id
            && pack.get('deathdagger') === quest[0]!.id
        ) {
            pool = standard;
            blackDaggerPreference = true;
        }
    }
    if (pool.length === 1) {
        const item = pool[0]!;
        if (!item.alias) throw new Error(`equipment names ${family}: ${requestedName} resolved without alias`);
        const packed = pack.get(item.alias);
        if (packed === undefined) throw new Error(`equipment names ${family}: missing obj.pack alias ${item.alias}`);
        if (packed !== item.id) throw new Error(`equipment names ${family}: obj.pack ${item.alias}=${packed} != decoded ${item.id}`);
        return {
            requested_name: requestedName,
            disposition: 'resolved',
            selected_name: item.name ?? requestedName,
            alias: item.alias,
            id: item.id,
            wear_position: item.wear_position,
            disambiguation: blackDaggerPreference
                ? 'prefer_standard_pack_alias_black_dagger'
                : exact.length !== 1
                    ? 'exclude_bank_note_prefer_wearable'
                    : undefined,
        };
    }
    if (pool.length === 0) {
        return {
            requested_name: requestedName,
            disposition: 'absent',
            absent_class: exact.length === 0 ? 'no_exact_selected_match' : 'exact_match_ineligible',
            candidates: exact.length > 0
                ? exact.map((item) => ({
                    alias: item.alias ?? '',
                    id: item.id,
                    name: item.name ?? requestedName,
                    wear_position: item.wear_position,
                    note: isBankNoteRow(item) ? 'bank_note' : 'filtered_out',
                }))
                : notes.map((item) => ({
                    alias: item.alias ?? '',
                    id: item.id,
                    name: item.name ?? requestedName,
                    wear_position: item.wear_position,
                    note: 'bank_note_only',
                })),
        };
    }
    return {
        requested_name: requestedName,
        disposition: 'ambiguous',
        candidates: pool.map((item) => ({
            alias: item.alias ?? '',
            id: item.id,
            name: item.name ?? requestedName,
            wear_position: item.wear_position,
            note: 'wearable_candidate',
        })),
    };
}

export function extractEquipmentNamesFacts(items: SelectedItemRow[], objPack: Map<string, number>, repoRoot: string = root, revision = 289): EquipmentNamesFacts {
    const pack = requireObjPack(objPack);
    const curated = loadEquipmentNamesCurated(repoRoot);
    const out: EquipmentNamesFacts = {
        exact_name_join: revision === 377 ? {
            ...EQUIPMENT_EXACT_NAME_JOIN,
            bolts: { ...EQUIPMENT_EXACT_NAME_JOIN.bolts, note: 'Frozen bolt names are exact queries. Each row derives its selected identity or absence; generic Bolts / Bolts(p) are never substituted for a tier name.' },
        } : EQUIPMENT_EXACT_NAME_JOIN,
        bows: [],
        crossbows: [],
        darts: [],
        arrows: [],
        bolts: [],
        melee_weapons: [],
        staffs: [],
    };
    for (const family of EQUIPMENT_FAMILY_ORDER) {
        const entries = curated.families[family].map((requestedName) => joinEquipmentName(family, requestedName, items, pack));
        const ambiguous = entries.filter((entry) => entry.disposition === 'ambiguous');
        if (ambiguous.length > 0 && revision !== 377) {
            throw new Error(`equipment names ${family}: ambiguous joins ${ambiguous.map((row) => row.requested_name).join(', ')}`);
        }
        out[family] = entries;
    }
    return out;
}
export function extractAutocastControls(content: string) {
    const interfaces = parsePack(fs.readFileSync(path.join(content, 'pack/interface.pack'), 'utf8'));
    const varps = parsePack(fs.readFileSync(path.join(content, 'pack/varp.pack'), 'utf8'));
    const required = (table: Map<string, number>, name: string) => {
        const id = table.get(name);
        if (id === undefined) throw new Error(`autocast: missing ${name}`);
        return id;
    };
    return {
        staff_tab_root: required(interfaces, 'combat_staff_2'),
        spell_text_component: required(interfaces, 'combat_staff_2:auto_spell'),
        spell_panel_root: required(interfaces, 'staff_spells'),
        choose_com: required(interfaces, 'combat_staff_2:auto_choose'),
        toggle_com: required(interfaces, 'combat_staff_2:auto_toggle'),
        spell_grid_base: required(interfaces, 'staff_spells:ssb0'),
        magic_varp: required(varps, 'attackstyle_magic'),
        selected_value: 2,
        armed_value: 3,
    };
}
export function parsePrayerConstants(text: string) {
    const names = new Set<string>();
    for (const raw of text.split(/\r?\n/)) {
        const line = raw.trim();
        if (!line.startsWith('^prayer_')) continue;
        const eq = line.indexOf('=');
        if (eq <= 0) continue;
        names.add(line.slice(1, eq).trim());
    }
    if (names.size === 0) throw new Error('prayer: missing prayers.constant entries');
    return names;
}

/** prayer.if section name → transmitted varp alias from pushvar wiring. */
export function parsePrayerInterface(text: string) {
    const out = new Map<string, string>();
    let current: string | null = null;
    for (const raw of text.split(/\r?\n/)) {
        const line = raw.trim();
        if (line.startsWith('[') && line.endsWith(']')) {
            current = line.slice(1, -1);
            continue;
        }
        if (!current || !line.startsWith('script1op1=pushvar,')) continue;
        const alias = line.slice('script1op1=pushvar,'.length);
        if (!alias) throw new Error(`prayer.if: empty pushvar in ${current}`);
        if (out.has(current)) throw new Error(`prayer.if: duplicate section ${current}`);
        out.set(current, alias);
    }
    return out;
}

export function extractPrayerFacts(content: string) {
    const interfaces = parsePack(fs.readFileSync(path.join(content, 'pack/interface.pack'), 'utf8'));
    const varps = parsePack(fs.readFileSync(path.join(content, 'pack/varp.pack'), 'utf8'));
    const constants = parsePrayerConstants(
        fs.readFileSync(path.join(content, 'scripts/skill_prayer/configs/prayers.constant'), 'utf8'),
    );
    const ifVarps = parsePrayerInterface(
        fs.readFileSync(path.join(content, 'scripts/skill_prayer/interfaces/prayer.if'), 'utf8'),
    );
    const prayers: {
        name: string;
        level: number;
        source_row: string;
        prayer_constant: string;
        button_com: number;
        com_alias: string;
        varp: number;
        varp_alias: string;
    }[] = [];
    const seenName = new Set<string>();
    const seenCom = new Set<number>();
    const seenVarp = new Set<number>();
    const seenConstant = new Set<string>();
    for (const parsed of parseRows(
        fs.readFileSync(path.join(content, 'scripts/skill_prayer/configs/prayers.dbrow'), 'utf8'),
    )) {
        const prayerRaw = parsed.values.prayer?.[0]?.[0];
        if (!prayerRaw) continue;
        const constant = prayerRaw.startsWith('^') ? prayerRaw.slice(1) : prayerRaw;
        if (!constants.has(constant)) throw new Error(`${parsed.name}: unknown prayer constant ${constant}`);
        if (seenConstant.has(constant)) throw new Error(`${parsed.name}: duplicate prayer constant ${constant}`);
        seenConstant.add(constant);
        const varpAlias = ifVarps.get(constant);
        if (!varpAlias) throw new Error(`${parsed.name}: missing prayer.if pushvar for ${constant}`);
        const varpId = varps.get(varpAlias);
        if (varpId === undefined) throw new Error(`${parsed.name}: missing varp.pack entry ${varpAlias}`);
        const comAlias = `prayer:${constant}`;
        const buttonCom = interfaces.get(comAlias);
        if (buttonCom === undefined) throw new Error(`${parsed.name}: missing interface.pack entry ${comAlias}`);
        const name = required(parsed.values, 'name', parsed.name);
        const level = integer(required(parsed.values, 'level', parsed.name), parsed.name);
        if (seenName.has(name)) throw new Error(`${parsed.name}: duplicate prayer name ${name}`);
        if (seenCom.has(buttonCom)) throw new Error(`${parsed.name}: duplicate button com ${buttonCom}`);
        if (seenVarp.has(varpId)) throw new Error(`${parsed.name}: duplicate varp ${varpId}`);
        seenName.add(name);
        seenCom.add(buttonCom);
        seenVarp.add(varpId);
        prayers.push({
            name,
            level,
            source_row: parsed.name,
            prayer_constant: constant,
            button_com: buttonCom,
            com_alias: comAlias,
            varp: varpId,
            varp_alias: varpAlias,
        });
    }
    if (prayers.length === 0) throw new Error('prayer: missing prayer rows');
    prayers.sort((a, b) => a.button_com - b.button_com || a.name.localeCompare(b.name));
    return { prayers };
}

export function extractDuelControls(content: string) {
    const interfaces = parsePack(fs.readFileSync(path.join(content, 'pack/interface.pack'), 'utf8'));
    const varps = parsePack(fs.readFileSync(path.join(content, 'pack/varp.pack'), 'utf8'));
    const required = (table: Map<string, number>, name: string) => {
        const id = table.get(name);
        if (id === undefined) throw new Error(`duel: missing ${name}`);
        return id;
    };
    return {
        select_modal: required(interfaces, 'duel_select_type'),
        confirm_modal: required(interfaces, 'duel_confirm'),
        win_modal: required(interfaces, 'duel_win'),
        select_accept: required(interfaces, 'duel_select_type:accept'),
        confirm_accept: required(interfaces, 'duel_confirm:accept'),
        select_partner: required(interfaces, 'duel_select_type:otherplayer'),
        select_status: required(interfaces, 'duel_select_type:status'),
        confirm_status: required(interfaces, 'duel_confirm:status'),
        select_mine: required(interfaces, 'duel_select_type:inv'),
        select_theirs: required(interfaces, 'duel_select_type:otherinv'),
        confirm_mine: required(interfaces, 'duel_confirm:inv'),
        confirm_theirs: required(interfaces, 'duel_confirm:otherinv'),
        obstacles: required(interfaces, 'duel_select_type:obstacles'),
        options_varp: required(varps, 'dueloptions'),
    };
}
function parseNamedConstant(text: string, name: string) {
    const prefix = `^${name}`;
    for (const raw of text.split(/\r?\n/)) {
        const line = raw.trim();
        if (!line.startsWith(prefix)) continue;
        const eq = line.indexOf('=');
        if (eq <= 0) continue;
        const parsed = Number(line.slice(eq + 1).trim());
        if (!Number.isInteger(parsed) || parsed <= 0) throw new Error(`special: bad ${name} in ${line}`);
        return parsed;
    }
    throw new Error(`special: missing ${name}`);
}
function paramTruthy(value: number | string | undefined) {
    return value === 1 || value === '1';
}
export function extractSpecialControls(content: string, items: ObjType[]) {
    const interfaces = parsePack(fs.readFileSync(path.join(content, 'pack/interface.pack'), 'utf8'));
    const varps = parsePack(fs.readFileSync(path.join(content, 'pack/varp.pack'), 'utf8'));
    const params = parsePack(fs.readFileSync(path.join(content, 'pack/param.pack'), 'utf8'));
    const required = (table: Map<string, number>, name: string) => {
        const id = table.get(name);
        if (id === undefined) throw new Error(`special: missing ${name}`);
        return id;
    };
    const specwepParam = required(params, 'specwep');
    const saEnergyParam = required(params, 'sa_energy');
    const maxEnergy = parseNamedConstant(fs.readFileSync(path.join(content, 'scripts/skill_combat/configs/combat.constant'), 'utf8'), 'sa_max_energy');
    const bars: { root: string; root_id: number; bar: number }[] = [];
    for (const [name, id] of interfaces) {
        if (name.includes(':') || !name.startsWith('combat_')) continue;
        const bar = interfaces.get(`${name}:specbar`);
        if (bar === undefined) continue;
        bars.push({ root: name, root_id: id, bar });
    }
    bars.sort((a, b) => a.root_id - b.root_id || a.bar - b.bar);
    if (bars.length === 0) throw new Error('special: no combat spec bars');
    const weapons: { alias: string; id: number; name: string; cost: number }[] = [];
    for (const item of items) {
        if (!item.debugname || !item.name) continue;
        const itemParams = item.params;
        if (!itemParams) continue;
        if (!paramTruthy(itemParams.get(specwepParam))) continue;
        const cost = itemParams.get(saEnergyParam);
        // specwep without sa_energy is a self-buff (Dragon battleaxe, Excalibur).
        if (typeof cost !== 'number' || !Number.isInteger(cost) || cost <= 0) continue;
        weapons.push({ alias: item.debugname, id: item.id, name: item.name, cost });
    }
    weapons.sort((a, b) => a.id - b.id || a.alias.localeCompare(b.alias));
    if (weapons.length === 0) throw new Error('special: no sa_energy spec weapons');
    return {
        energy_varp: required(varps, 'sa_energy'),
        armed_varp: required(varps, 'sa_attack'),
        armed_value: 1,
        max_energy: maxEnergy,
        arm_confirm_ticks: 2,
        bars,
        weapons,
    };
}
function parseTeleCoord(value: string, label: string) {
    const parts = value.split('_');
    if (parts.length !== 5) throw new Error(`${label}: bad tele_coord ${value}`);
    const plane = integer(parts[0], `${label} plane`);
    const mx = integer(parts[1], `${label} mx`);
    const mz = integer(parts[2], `${label} mz`);
    const lx = integer(parts[3], `${label} lx`);
    const lz = integer(parts[4], `${label} lz`);
    return { packed: value, plane, x: mx * 64 + lx, z: mz * 64 + lz };
}
function titleDest(spell: string) {
    const key = spell.startsWith('^') ? spell.slice(1) : spell;
    if (!key.endsWith('_teleport')) throw new Error(`teleport: ${spell} is not a *_teleport spell`);
    const dest = key.slice(0, -'_teleport'.length);
    if (!dest) throw new Error(`teleport: empty dest in ${spell}`);
    return { key, name: dest.charAt(0).toUpperCase() + dest.slice(1) };
}
export function extractTeleportSpells(content: string, items: ObjType[]) {
    const itemIds = new Map(items.filter((item) => item.debugname !== null).map((item) => [item.debugname as string, { id: item.id, name: item.name }]));
    const interfaces = parsePack(fs.readFileSync(path.join(content, 'pack/interface.pack'), 'utf8'));
    const teleports = [];
    for (const parsed of parseRows(fs.readFileSync(path.join(content, 'scripts/skill_magic/configs/magic_spells.dbrow'), 'utf8'))) {
        const packed = parsed.values.tele_coord?.[0]?.[0];
        if (!packed) continue;
        const spellConst = required(parsed.values, 'spell', parsed.name);
        const { key, name } = titleDest(spellConst);
        const componentId = interfaces.get(`magic:${key}`);
        if (componentId === undefined) throw new Error(`teleport: missing magic:${key}`);
        const coord = parseTeleCoord(packed, parsed.name);
        const members = required(parsed.values, 'members', parsed.name) === 'true';
        teleports.push({
            name,
            source_row: parsed.name,
            spell: key,
            component_id: componentId,
            members,
            level: integer(required(parsed.values, 'levelrequired', parsed.name), parsed.name),
            runes: runeCosts(parsed.values, parsed.name, itemIds),
            experience: integer(required(parsed.values, 'experience', parsed.name), parsed.name),
            tele_coord: coord.packed,
            x: coord.x,
            z: coord.z,
            plane: coord.plane,
        });
    }
    if (teleports.length === 0) throw new Error('teleport: no tele_coord rows');
    return teleports;
}

type ObjSection = { name?: string; cost?: number; params: Map<string, string> };

export function parseObjSections(text: string) {
    const sections = new Map<string, ObjSection>();
    let current: string | null = null;
    for (const raw of text.split(/\r?\n/)) {
        const line = raw.trim();
        if (!line || line.startsWith('//')) continue;
        if (line.startsWith('[') && line.endsWith(']')) {
            current = line.slice(1, -1);
            sections.set(current, { params: new Map() });
            continue;
        }
        if (!current) continue;
        const entry = sections.get(current)!;
        if (line.startsWith('name=')) entry.name = line.slice('name='.length);
        else if (line.startsWith('cost=')) entry.cost = integer(line.slice('cost='.length), `${current} cost`);
        else if (line.startsWith('param=')) {
            const [, rest] = line.split('=', 2);
            const comma = rest.indexOf(',');
            if (comma <= 0) throw new Error(`${current}: bad param ${line}`);
            entry.params.set(rest.slice(0, comma), rest.slice(comma + 1));
        }
    }
    return sections;
}

export function parseIdentifyHerbPairs(text: string) {
    const pairs: { unidAlias: string; idAlias: string }[] = [];
    const re = /\[opheld1,(unidentified_\w+)\][^\n]*\n~attempt_identify_herb\((\w+),/g;
    for (const match of text.matchAll(re)) {
        pairs.push({ unidAlias: match[1], idAlias: match[2] });
    }
    if (pairs.length === 0) throw new Error('herbs: no identify.rs2 pairs');
    return pairs;
}


/** Default Herblore level from local `identify.param` content. */
export function identifiedHerbLevelDefault(content: string) {
    const defs = parseParamDefinitions(
        fs.readFileSync(path.join(content, 'scripts/skill_herblore/configs/identifying/identify.param'), 'utf8'),
    );
    const level = defs.get('identified_herb_level');
    if (!level?.default) throw new Error('herbs: missing identified_herb_level default in identify.param');
    return integer(level.default, 'identified_herb_level default');
}

/** Script-facing herb key from cleaned display name (reference herbs.ts parity). */
export function herbKeyFromName(name: string) {
    const lower = name.toLowerCase();
    if (lower.endsWith(' leaf')) return lower.slice(0, -' leaf'.length);
    if (lower === 'dwarf weed' || lower === 'snake weed') return lower;
    if (lower.endsWith(' weed')) return lower.slice(0, -' weed'.length);
    return lower;
}

export function extractHerbFacts(content: string, items: ObjType[], revision = 289) {
    const itemIds = new Map(items.filter((item) => item.debugname !== null).map((item) => [item.debugname as string, { id: item.id, name: item.name }]));
    const levelDefault = identifiedHerbLevelDefault(content);
    const herbsObj = parseObjSections(fs.readFileSync(path.join(content, 'scripts/skill_herblore/configs/herbs.obj'), 'utf8'));
    if (revision === 377) {
        const unpacked = parseObjSections(fs.readFileSync(path.join(content, unpacked377HerbsFile), 'utf8'));
        for (const [alias, section] of unpacked) {
            if (herbsObj.has(alias)) throw new Error(`herbs: duplicate selected obj section ${alias}`);
            herbsObj.set(alias, section);
        }
    }
    const identify = fs.readFileSync(path.join(content, 'scripts/skill_herblore/scripts/identifying/identify.rs2'), 'utf8');
    const pairs = parseIdentifyHerbPairs(identify);
    const herbs: { key: string; name: string; id: number; unidId: number; level: number }[] = [];
    for (const { unidAlias, idAlias } of pairs) {
        const section = herbsObj.get(idAlias);
        if (!section) throw new Error(`herbs: missing herbs.obj section ${idAlias}`);
        const display = section.name;
        if (!display) throw new Error(`herbs: ${idAlias} missing name`);
        const levelRaw = section.params.get('identified_herb_level');
        const level = levelRaw !== undefined
            ? integer(levelRaw, `${idAlias} identified_herb_level`)
            : levelDefault;
        const cleaned = itemIds.get(idAlias);
        const unid = itemIds.get(unidAlias);
        if (!cleaned) throw new Error(`herbs: unknown identified obj ${idAlias}`);
        if (!unid) throw new Error(`herbs: unknown unidentified obj ${unidAlias}`);
        if (!cleaned.name) throw new Error(`herbs: ${idAlias} missing obj display name`);
        herbs.push({
            key: herbKeyFromName(display),
            name: display,
            id: cleaned.id,
            unidId: unid.id,
            level,
        });
    }
    return { herbs, herb_level_default: levelDefault };
}

type DropBlock = { type: string; name: string; body: string };
type DropToken = { value: string; optional: boolean };

function parseDropBlocks(content: string, revision: number) {
    const blocks = new Map<string, DropBlock>();
    for (const relative of dropContentFilesForRevision(revision).filter((file) => file.endsWith('.rs2'))) {
        const file = path.join(content, relative);
        if (!fs.existsSync(file)) throw new Error(`missing content input ${relative}`);
        let current: DropBlock | null = null;
        let lines: string[] = [];
        const flush = () => {
            if (!current) return;
            const key = `${current.type}:${current.name}`;
            if (blocks.has(key)) throw new Error(`duplicate drop block ${key}`);
            current.body = lines.join('\n');
            blocks.set(key, current);
            lines = [];
        };
        for (const raw of fs.readFileSync(file, 'utf8').split(/\r?\n/)) {
            const head = /^\[([a-z0-9_]+)\s*,\s*([a-z0-9_]+)\]/.exec(raw.trim());
            if (head) {
                flush();
                current = { type: head[1], name: head[2], body: '' };
                if (revision === 377) {
                    const inline = raw.trim().slice(head[0].length).trim();
                    if (inline.startsWith('@')) lines.push(inline);
                }
            } else if (current) {
                lines.push(raw);
            }
        }
        flush();
    }
    return blocks;
}

function parseDropNpcs(content: string, revision: number) {
    const rows = new Map<string, { name?: string; death_drop?: string }>();
    for (const relative of dropContentFilesForRevision(revision).filter((file) => file.endsWith('.npc'))) {
        const file = path.join(content, relative);
        if (!fs.existsSync(file)) throw new Error(`missing content input ${relative}`);
        let current: string | null = null;
        for (const raw of fs.readFileSync(file, 'utf8').split(/\r?\n/)) {
            const line = raw.trim();
            const head = /^\[([a-z0-9_]+)\]$/.exec(line);
            if (head) {
                current = head[1];
                if (rows.has(current)) throw new Error(`duplicate npc config ${current}`);
                rows.set(current, {});
            } else if (current && line.startsWith('name=')) {
                rows.get(current)!.name = line.slice('name='.length);
            } else if (current && line.startsWith('param=death_drop,')) {
                rows.get(current)!.death_drop = line.slice('param=death_drop,'.length).split(',')[0].trim();
            }
        }
    }
    return rows;
}

function dropTokens(body: string) {
    const tokens: DropToken[] = [];
    for (const match of body.matchAll(/obj_add\s*\(\s*npc_coord\s*,\s*(~?[a-z0-9_]+)/g)) {
        tokens.push({ value: match[1], optional: false });
    }
    for (const match of body.matchAll(/return\s*\(\s*(~?[a-z0-9_]+)/g)) {
        tokens.push({ value: match[1], optional: false });
    }
    // Some tables stage an item in a local before returning the variable.
    // Non-item assignments are ignored only when they do not join to ObjType.
    for (const match of body.matchAll(/=\s*([a-z][a-z0-9_]*)\s*;/g)) {
        tokens.push({ value: match[1], optional: true });
    }
    return tokens;
}

const SHOP_INVENTORY_SOURCES = [
    { inv: 'adventurershop', file: 'scripts/areas/area_ardougne_east/configs/ardougne_east.inv' },
    { inv: 'magearena_runeshop', file: 'scripts/areas/area_mage_arena/configs/mage_arena.inv' },
    { inv: 'magicshop', file: 'scripts/areas/area_port_sarim/configs/port_sarim.inv' },
    { inv: 'runeshop', file: 'scripts/areas/area_varrock/configs/varrock.inv' },
    { inv: 'archeryshop', file: 'scripts/areas/area_varrock/configs/varrock.inv' },
    { inv: 'archeryshop2', file: 'scripts/areas/area_catherby/configs/catherby.inv' },
    { inv: 'fishingshop', file: 'scripts/areas/area_port_sarim/configs/port_sarim.inv' },
    { inv: 'fishingshop2', file: 'scripts/areas/area_catherby/configs/catherby.inv' },
    { inv: 'shilofishingshop', file: 'scripts/areas/area_shilo/configs/shilo.inv' },
    { inv: 'axeshop', file: 'scripts/areas/area_lumbridge/configs/lumbridge.inv' },
    { inv: 'pickaxeshop', file: 'scripts/areas/area_falador/configs/dwarven_mine.inv' },
    { inv: 'magicguildshop', file: 'scripts/areas/area_yanille/configs/magic_guild/magic_guild.inv' },
] as const;

const SHOP_KEEPER_SOURCES = [
    { file: 'scripts/areas/area_ardougne_east/configs/ardougne_east.npc', configs: ['aemad', 'kortan'] },
    { file: 'scripts/areas/area_mage_arena/configs/mage_arena.npc', configs: ['magearena_runeshop'] },
    { file: 'scripts/areas/area_port_sarim/configs/port_sarim.npc', configs: ['betty', 'gerrant'] },
    { file: 'scripts/areas/area_varrock/configs/varrock.npc', configs: ['aubury', 'lowe'] },
    { file: 'scripts/areas/area_catherby/configs/catherby.npc', configs: ['hickton', 'harry'] },
    { file: 'scripts/areas/area_shilo/configs/shilo.npc', configs: ['shilofishowner'] },
    { file: 'scripts/areas/area_lumbridge/configs/lumbridge.npc', configs: ['bob'] },
    { file: 'scripts/areas/area_falador/configs/dwarven_mine.npc', configs: ['nurmof'] },
    { file: 'scripts/areas/area_yanille/configs/magic_guild/magic_guild.npc', configs: ['magic_store_owner'] },
] as const;

type ShopItemSource = {
    alias: string | null;
    name: string | null;
    cost: number;
    stackable: boolean;
    members: boolean;
};
type ShopInventory = {
    inv: string;
    scope: string;
    allstock: boolean;
    stock: { alias: string; baseline_qty: number; restock_delta: number }[];
};
type ParsedShopKeeper = {
    config: string;
    name: string;
    owned_shop: string;
    sell: number;
    buy: number;
    delta: number;
    title: string;
};
type ShopItemFact = {
    obj: string;
    name: string;
    baseline: number;
    restockTicks: number;
    cost: number;
    stackable: boolean;
    members: boolean;
};
type ShopRecordFact = {
    inv: string;
    title: string;
    keepers: string[];
    keeper_configs: string[];
    sell: number;
    buy: number;
    delta: number;
    scope: string;
    allstock: boolean;
    items: ShopItemFact[];
};

function parseShopInventory(text: string, inv: string): ShopInventory {
    let inSection = false;
    let scope = '';
    let allstock = false;
    for (const raw of text.split(/\r?\n/)) {
        const line = raw.trim();
        if (line === `[${inv}]`) {
            inSection = true;
            continue;
        }
        if (line.startsWith('[') && line.endsWith(']')) {
            if (inSection) break;
            continue;
        }
        if (!inSection) continue;
        if (line.startsWith('scope=')) scope = line.slice('scope='.length);
        else if (line.startsWith('allstock=')) allstock = line.slice('allstock='.length) === 'yes';
    }
    return { inv, scope, allstock, stock: parseInvShopStock(text, inv) };
}

function extractShopFacts(content: string, items: readonly ShopItemSource[], revision = 289) {
    const keepers: ParsedShopKeeper[] = [];
    const selectedNpcs = revision === 377
        ? fs.readFileSync(path.join(content, 'scripts/_unpack/377/all.npc'), 'utf8')
        : undefined;
    const selectedInventories = revision === 377
        ? contentTreeFiles(content, 'scripts', '.inv').map((file) => fs.readFileSync(path.join(content, file), 'utf8'))
        : undefined;
    for (const source of SHOP_KEEPER_SOURCES) {
        const text = selectedNpcs ?? fs.readFileSync(path.join(content, source.file), 'utf8');
        for (const config of source.configs) {
            const values = parseNpcSection(text, config);
            const required = (key: string) => {
                const value = values[key];
                if (value === undefined || value === '') throw new Error(`${config}: missing ${key}`);
                return value;
            };
            keepers.push({
                config,
                name: values.name || config,
                owned_shop: required('owned_shop'),
                sell: integer(required('shop_sell_multiplier'), `${config}: shop_sell_multiplier`),
                buy: integer(required('shop_buy_multiplier'), `${config}: shop_buy_multiplier`),
                delta: integer(required('shop_delta'), `${config}: shop_delta`),
                title: required('shop_title'),
            });
        }
    }
    const itemByAlias = new Map(items.filter((item) => item.alias !== null).map((item) => [item.alias as string, item]));
    const shops: ShopRecordFact[] = SHOP_INVENTORY_SOURCES.map(({ inv, file }) => {
        const selected = selectedInventories?.flatMap((text) =>
            Array.from(text.matchAll(new RegExp(`^\\[${inv}\\][\\t ]*\\r?$`, 'gm')), () => text));
        if (selected && selected.length !== 1) throw new Error(`${inv}: expected one selected inventory definition, found ${selected.length}`);
        const inventory = parseShopInventory(selected ? selected[0]! : fs.readFileSync(path.join(content, file), 'utf8'), inv);
        const owners = keepers.filter((keeper) => keeper.owned_shop === inv);
        const first = owners[0];
        if (!first) throw new Error(`${inv}: no keeper owns this inventory`);
        const stock = inventory.stock.map((row) => {
            const item = itemByAlias.get(row.alias);
            if (!item?.name) throw new Error(`${inv}: missing named item ${row.alias}`);
            return {
                obj: row.alias,
                name: item.name,
                baseline: row.baseline_qty,
                restockTicks: row.restock_delta,
                cost: item.cost,
                stackable: item.stackable,
                members: item.members,
            };
        });
        return {
            inv,
            title: first.title,
            keepers: owners.map((keeper) => keeper.name),
            keeper_configs: owners.map((keeper) => keeper.config),
            sell: first.sell,
            buy: first.buy,
            delta: first.delta,
            scope: inventory.scope,
            allstock: inventory.allstock,
            items: stock,
        };
    });
    return shops;
}

export function parseInvShopStock(text: string, shopName: string) {
    const stock: { alias: string; baseline_qty: number; restock_delta: number }[] = [];
    let inSection = false;
    for (const raw of text.split(/\r?\n/)) {
        const line = raw.trim();
        if (line === `[${shopName}]`) {
            inSection = true;
            continue;
        }
        if (line.startsWith('[') && line.endsWith(']')) {
            if (inSection) break;
            continue;
        }
        if (!inSection || !line.startsWith('stock')) continue;
        const eq = line.indexOf('=');
        if (eq <= 0) throw new Error(`${shopName}: bad stock line ${line}`);
        const parts = line.slice(eq + 1).split(',');
        if (parts.length < 3) throw new Error(`${shopName}: bad stock line ${line}`);
        stock.push({
            alias: parts[0],
            baseline_qty: integer(parts[1], line),
            restock_delta: integer(parts[2], line),
        });
    }
    if (stock.length === 0) throw new Error(`${shopName}: missing stock rows`);
    return stock;
}

function parseNpcConfigSections(text: string) {
    const sections = new Map<string, Record<string, string>>();
    let current: string | null = null;
    for (const raw of text.split(/\r?\n/)) {
        const line = raw.trim();
        if (line.startsWith('[') && line.endsWith(']')) {
            current = line.slice(1, -1);
            sections.set(current, {});
            continue;
        }
        if (!current || !line.includes('=')) continue;
        const entry = sections.get(current)!;
        if (line.startsWith('param=')) {
            const comma = line.indexOf(',', 'param='.length);
            if (comma <= 0) throw new Error(`${current}: bad param ${line}`);
            entry[line.slice('param='.length, comma)] = line.slice(comma + 1);
            continue;
        }
        const eq = line.indexOf('=');
        entry[line.slice(0, eq)] = line.slice(eq + 1);
    }
    return sections;
}

export function parseNpcSection(text: string, alias: string) {
    const section = parseNpcConfigSections(text).get(alias);
    if (!section) throw new Error(`npc config: missing [${alias}]`);
    return section;
}

export function parseQuestEnumEntry(text: string, questName: string) {
    let inQuestNames = false;
    let found: string | null = null;
    for (const raw of text.split(/\r?\n/)) {
        const line = raw.trim();
        if (!line || line.startsWith('//')) continue;
        if (line.startsWith('[') && line.endsWith(']')) {
            const section = line.slice(1, -1);
            if (inQuestNames && section !== 'quest_names_enum') break;
            inQuestNames = section === 'quest_names_enum';
            continue;
        }
        if (!inQuestNames || !line.startsWith('val=')) continue;
        const eq = line.indexOf('=');
        const comma = line.indexOf(',', eq + 1);
        if (comma <= eq) throw new Error(`quest.enum: malformed val line ${line}`);
        const name = line.slice(comma + 1).trim();
        if (name !== questName) continue;
        if (found) throw new Error(`quest.enum: duplicate val for ${questName}`);
        found = line;
    }
    if (!found) throw new Error(`quest.enum: missing ${questName} in [quest_names_enum]`);
    return found;
}


export function parseLocSection(text: string, alias: string) {
    const sections = new Map<string, { name?: string }>();
    let current: string | null = null;
    for (const raw of text.split(/\r?\n/)) {
        const line = raw.trim();
        if (line.startsWith('[') && line.endsWith(']')) {
            current = line.slice(1, -1);
            sections.set(current, {});
            continue;
        }
        if (!current || !line.startsWith('name=')) continue;
        sections.get(current)!.name = line.slice('name='.length);
    }
    const section = sections.get(alias);
    if (!section?.name) throw new Error(`loc config: missing [${alias}] name`);
    return section;
}



export function extractNurmofEssenceFacts(content: string, items: ObjType[], npcs: NpcType[]) {
    const itemByAlias = new Map(items.filter((item) => item.debugname !== null).map((item) => [item.debugname as string, item]));
    const invRelative = 'scripts/areas/area_falador/configs/dwarven_mine.inv';
    const stock = parseInvShopStock(fs.readFileSync(path.join(content, invRelative), 'utf8'), 'pickaxeshop');
    const pickaxes = stock.map((row) => {
        const item = itemByAlias.get(row.alias);
        if (!item?.name) throw new Error(`pickaxeshop: missing named obj ${row.alias}`);
        return {
            alias: row.alias,
            id: item.id,
            name: item.name,
            base_cost: item.cost,
            shop_baseline_qty: row.baseline_qty,
        };
    });
    const npcConfig = parseNpcSection(
        fs.readFileSync(path.join(content, 'scripts/areas/area_falador/configs/dwarven_mine.npc'), 'utf8'),
        'nurmof',
    );
    if (!npcConfig.name || !npcConfig.owned_shop) throw new Error('nurmof: missing name or owned_shop');
    const nurmof = npcs.find((npc) => npc.debugname === 'nurmof');
    if (!nurmof?.name) throw new Error('nurmof: missing decoded npc');
    if (nurmof.name !== npcConfig.name) throw new Error(`nurmof: content/cache name join mismatch ${npcConfig.name}/${nurmof.name}`);
    const npcPack = parsePack(fs.readFileSync(path.join(content, 'pack/npc.pack'), 'utf8'));
    const packedNurmofId = npcPack.get('nurmof');
    if (packedNurmofId !== nurmof.id) throw new Error(`nurmof: pack/cache id join mismatch ${packedNurmofId}/${nurmof.id}`);
    const essenceMap = 'maps/m45_75.jm2';
    if (!fs.existsSync(path.join(content, essenceMap))) throw new Error(`missing ${essenceMap}`);
    const { mx: essence_mapsquare_mx, mz: essence_mapsquare_mz } = parseMapsquarePath(essenceMap);
    const portalAlias = 'blankrunestone_exit_portal';
    const locPack = parsePack(fs.readFileSync(path.join(content, 'pack/loc.pack'), 'utf8'));
    const minePortalLocId = locPack.get(portalAlias);
    if (minePortalLocId === undefined) throw new Error(`essence mine: missing loc config ${portalAlias}`);
    const portalPlacements = parseJm2LocPlacements(
        fs.readFileSync(path.join(content, essenceMap), 'utf8'),
        new Set([minePortalLocId]),
    );
    if (portalPlacements.length === 0) {
        throw new Error(`essence mine: no loc ${minePortalLocId} placements in ${essenceMap}`);
    }
    const auburyPacked = npcPack.get('aubury');
    const auburyNpc = npcs.find((npc) => npc.debugname === 'aubury');
    if (auburyPacked === undefined || !auburyNpc?.name || auburyPacked !== auburyNpc.id) {
        throw new Error(`aubury: pack/cache join mismatch ${auburyPacked}/${auburyNpc?.id}`);
    }
    const runecraftConstantPath = 'scripts/skill_runecraft/configs/runecraft.constant';
    const runecraftConstant = fs.readFileSync(path.join(content, runecraftConstantPath), 'utf8');
    const returnAnchorMatch = runecraftConstant.match(/^\^essence_mine_to_aubury\s*=\s*(\S+)/m);
    if (!returnAnchorMatch) throw new Error('runecraft.constant: missing ^essence_mine_to_aubury return anchor');
    const example = { x: 2880, z: 4800, plane: 0 };
    return {
        npc_alias: 'nurmof',
        npc_id: nurmof.id,
        npc_name: nurmof.name,
        shop_inv: npcConfig.owned_shop,
        pickaxes,
        essence_region: {
            mapsquare_mx: essence_mapsquare_mx,
            mapsquare_mz: essence_mapsquare_mz,
            predicate: `(x >> 6) === ${essence_mapsquare_mx} && (z >> 6) === ${essence_mapsquare_mz}`,
            mine_portal_loc_alias: portalAlias,
            mine_portal_loc_id: minePortalLocId,
            mine_portal_loc_placements: portalPlacements.length,
            example_inside: example,
        },
        aubury_travel: {
            note: 'Essence wizard Aubury entry/return hops are already packed in nav transport; not duplicated here.',
            npc_alias: 'aubury',
            npc_id: auburyNpc.id,
            npc_name: auburyNpc.name,
            already_packed: true,
            return_anchor_constant: '^essence_mine_to_aubury',
            return_anchor_coord: returnAnchorMatch[1],
            return_anchor_role: 'overworld_exit_after_mine_teleport',
        },
        curated_vendor_tactics: {
            label: 'curated',
            keeper: npcConfig.name,
            stand: { x: 2997, z: 9844, plane: 0 },
            bank_stand: { x: 3013, z: 3355, plane: 0 },
            hop_from: { x: 3019, z: 3449, plane: 0 },
            hop_loc: 'Trapdoor',
            hop_action: 'Climb-down',
        },
    };
}

export function extractFlourSixFacts(content: string, items: ObjType[]) {
    const itemByAlias = new Map(items.filter((item) => item.debugname !== null).map((item) => [item.debugname as string, item]));
    parseQuestEnumEntry(
        fs.readFileSync(path.join(content, 'scripts/general/configs/quest.enum'), 'utf8'),
        'Murder Mystery',
    );
    const locRelative = 'scripts/quests/quest_murder/configs/quest_murder.loc';
    const flourLoc = parseLocSection(fs.readFileSync(path.join(content, locRelative), 'utf8'), 'flourbarrel');
    const locPack = parsePack(fs.readFileSync(path.join(content, 'pack/loc.pack'), 'utf8'));
    const flourLocId = locPack.get('flourbarrel');
    if (flourLocId === undefined) throw new Error('flourbarrel: missing from pack/loc.pack');
    const pot = itemByAlias.get('pot_empty');
    const potFlour = itemByAlias.get('pot_flour');
    if (!pot?.name) throw new Error(`flour: pot_empty join ${JSON.stringify(pot)}`);
    if (!potFlour?.name) throw new Error(`flour: pot_flour join ${JSON.stringify(potFlour)}`);
    const objPack = parsePack(fs.readFileSync(path.join(content, 'pack/obj.pack'), 'utf8'));
    if (objPack.get('pot_empty') !== pot.id || objPack.get('pot_flour') !== potFlour.id) {
        throw new Error('flour: obj.pack ids disagree with decoded pot items');
    }
    const mapRelative = 'maps/m42_55.jm2';
    const { mx, mz } = parseMapsquarePath(mapRelative);
    const placements = parseJm2LocPlacements(fs.readFileSync(path.join(content, mapRelative), 'utf8'), new Set([flourLocId]));
    if (placements.length !== 1) {
        throw new Error(`flourbarrel: expected one ${mapRelative} LOC placement, got ${placements.length}`);
    }
    const placement = placements[0];
    const derivedBarrel = worldFromMapsquare(mx, mz, placement.lx, placement.lz, placement.plane);
    return {
        quest_name: 'Murder Mystery',
        pot: { alias: 'pot_empty', id: pot.id, name: pot.name },
        pot_flour: { alias: 'pot_flour', id: potFlour.id, name: potFlour.name },
        flour_barrel: {
            alias: 'flourbarrel',
            id: flourLocId,
            name: flourLoc.name!,
        },
        flour_barrel_object_tile: {
            ...derivedBarrel,
            role: 'loc_placement',
            mapsquare: `m${mx}_${mz}`,
            local: { lx: placement.lx, lz: placement.lz },
            loc_shape: placement.shape,
            loc_angle: placement.angle,
        },
        flour_barrel_approach_tile: {
            x: 2735,
            z: 3581,
            plane: 0,
            role: 'interaction_near',
            note: 'FlourCollector Reach.locOp near / recovery anchor; not the jm2 loc tile (object at z=3582).',
        },
        bank_tile: {
            x: 2725,
            z: 3491,
            plane: 0,
            note: 'Bank stand tile for Murder Mystery withdraw; not a loc placement row in selected content.',
        },
    };
}

export function extractDropFacts(content: string, items: ObjType[], npcs: NpcType[], revision = 289) {
    const targets: { alias: string; block: string; explicit?: boolean }[] = [
        { alias: 'giant', block: 'ai_queue3:giant' },
        { alias: 'mossgiant', block: 'ai_queue3:mossgiant' },
        { alias: 'firegiant', block: 'ai_queue3:firegiant' },
        { alias: 'green_dragon', block: 'ai_queue3:green_dragon' },
        { alias: 'kalphite_flyingqueen', block: 'ai_queue3:kalphite_flyingqueen', explicit: true },
    ];
    const blocks = parseDropBlocks(content, revision);
    const npcConfigs = parseDropNpcs(content, revision);
    const itemIds = new Map(items.filter((item) => item.debugname !== null).map((item) => [item.debugname as string, item]));
    const npcIds = new Map(npcs.filter((npc) => npc.debugname != null).map((npc) => [npc.debugname as string, npc]));

    const resolve = (key: string, deathDrop: string | undefined, seen: Set<string>, out: Set<string>) => {
        if (seen.has(key)) return;
        const block = blocks.get(key);
        if (!block) throw new Error(`missing drop block ${key}`);
        seen.add(key);
        const body = revision === 377 ? block.body.replace(/\/\/[^\n]*/g, '') : block.body;
        if (revision === 377) {
            for (const transfer of body.matchAll(/@([a-z0-9_]+)\s*;/g)) {
                resolve(`label:${transfer[1]}`, deathDrop, seen, out);
            }
        }
        for (const token of dropTokens(body)) {
            if (token.value.startsWith('~')) {
                resolve(`proc:${token.value.slice(1)}`, deathDrop, seen, out);
                continue;
            }
            if (token.value === 'npc_param' && !deathDrop) {
                throw new Error(`${key}: npc_param(death_drop) requires a configured death_drop`);
            }
            const alias = token.value === 'npc_param' ? deathDrop! : token.value;
            if (itemIds.has(alias)) {
                out.add(alias);
            } else if (alias.startsWith('cert_') && itemIds.has(alias.slice('cert_'.length))) {
                out.add(alias.slice('cert_'.length));
            } else if (!token.optional) {
                throw new Error(`${key}: unknown drop item ${alias}`);
            }
        }
    };

    return targets.map((target) => {
        const config = npcConfigs.get(target.alias);
        if (!config?.name) throw new Error(`${target.alias}: missing npc display name`);
        if (!config.death_drop && !target.explicit) throw new Error(`${target.alias}: missing death_drop`);
        const npc = npcIds.get(target.alias);
        if (!npc) throw new Error(`${target.alias}: missing decoded npc`);
        if (npc.name !== config.name) throw new Error(`${target.alias}: npc display mismatch ${npc.name}/${config.name}`);
        const aliases = new Set<string>();
        resolve(target.block, config.death_drop, new Set(), aliases);
        const rows = [...aliases]
            .map((alias) => {
                const item = itemIds.get(alias)!;
                if (!item.name) throw new Error(`${target.alias}: ${alias} has no display name`);
                return { alias, id: item.id, name: item.name };
            })
            .sort((a, b) => a.name.localeCompare(b.name) || a.alias.localeCompare(b.alias));
        const displayNames = [...new Set(rows.map((item) => item.name))].sort((a, b) => a.localeCompare(b));
        if (rows.length === 0 || displayNames.length === 0) throw new Error(`${target.alias}: empty drop table`);
        return {
            npc_alias: target.alias,
            npc_id: npc.id,
            name: config.name,
            source_block: target.block,
            items: rows,
            display_names: displayNames,
        };
    });
}

/** One selected `param=` line. The value stays the raw string, in file order. */
export type TrailParam = { key: string; value: string };
/** One membership row: an enum alias or a casket those rows name. `access` marks the constrained clue alias. */
export type TrailMembershipRow = { alias: string; id: number; role: 'clue' | 'casket'; params: TrailParam[]; access?: 'constrained' };
/** One challenge answer. The raw param string, not a coerced number. */
export type TrailChallengeAnswer = { alias: string; id: number; answer: string };
export type TrailFacts = { rows: TrailMembershipRow[]; challenge_answers: TrailChallengeAnswer[] };

const TRAIL_TIERS = ['easy', 'medium', 'hard'] as const;
const TRAIL_CONFIG_DIR = 'scripts/minigames/game_trail/configs';
const TRAIL_CASKET_RELATIVE = `${TRAIL_CONFIG_DIR}/trail_casket.obj`;
/** The clue whose access fact is constrained. */
const TRAIL_CONSTRAINED = { alias: 'trail_clue_hard_sextant028' };

/**
 * Trail obj blocks by alias. Only `param=` lines are selected. Repeated keys stay
 * a list in file order; no last-write-wins. This is not `parseConfigSections`.
 */
export function parseTrailObjBlocks(text: string) {
    const blocks = new Map<string, TrailParam[]>();
    let current: TrailParam[] | null = null;
    for (const raw of text.split(/\r?\n/)) {
        const line = raw.trim();
        if (!line || line.startsWith('//')) continue;
        if (line.startsWith('[') && line.endsWith(']')) {
            const alias = line.slice(1, -1);
            if (blocks.has(alias)) throw new Error(`${alias}: duplicate trail obj block`);
            current = [];
            blocks.set(alias, current);
            continue;
        }
        if (!current) continue;
        const eq = line.indexOf('=');
        if (eq <= 0 || line.slice(0, eq) !== 'param') continue;
        const value = line.slice(eq + 1);
        const comma = value.indexOf(',');
        current.push({ key: comma < 0 ? value : value.slice(0, comma), value: comma < 0 ? '' : value.slice(comma + 1) });
    }
    return blocks;
}

/** `val=<index>,<alias>` rows of one trail enum, in file order. */
export function parseTrailEnumAliases(text: string) {
    const aliases: string[] = [];
    for (const raw of text.split(/\r?\n/)) {
        const line = raw.trim();
        if (!line || line.startsWith('//') || !line.startsWith('val=')) continue;
        const comma = line.indexOf(',');
        if (comma < 0) throw new Error(`${line}: malformed trail enum row`);
        const alias = line.slice(comma + 1).trim();
        if (!alias) throw new Error(`${line}: malformed trail enum row`);
        aliases.push(alias);
    }
    return aliases;
}

function trailJoin(objPack: Map<string, number>, itemIds: Map<string, { id: number; name: string | null }>, alias: string, label: string) {
    const packed = objPack.get(alias);
    if (packed === undefined) throw new Error(`${label}: pack/obj.pack lacks ${alias}`);
    const decoded = itemIds.get(alias);
    if (!decoded) throw new Error(`${label}: ${alias} has no decoded item row`);
    if (decoded.id !== packed) throw new Error(`${label}: pack/obj.pack ${alias}=${packed} disagrees with decoded item id ${decoded.id}`);
    return { alias, id: packed };
}

/**
 * Trail membership on the selected content. Clue rows are the three enum joins
 * (`trail_easy.enum`, `trail_medium.enum`, `trail_hard.enum`) against the matching
 * obj block and `pack/obj.pack`, in enum order. Casket rows are the unique
 * `param=trail_casket` aliases those rows name, in first-named order. A named
 * casket needs no obj block. Every join must equal the decoded item id.
 * Challenge answers are a sibling list, not membership rows.
 */
export function extractTrailFacts(content: string, items: ObjType[], revision = 289): TrailFacts {
    const itemIds = new Map(items.filter((item) => item.debugname !== null).map((item) => [item.debugname as string, { id: item.id, name: item.name, params: item.params }]));
    const objPack = parsePack(requireGatherText(content, 'pack/obj.pack'));
    if (objPack.size === 0) throw new Error('pack/obj.pack: required file missing ids');
    const answerParamId = revision === 377 ? parsePack(requireGatherText(content, 'pack/param.pack')).get('trail_challenge_answer') : undefined;
    if (revision === 377 && answerParamId === undefined) throw new Error('trails: selected param pack lacks trail_challenge_answer');
    const blocks = new Map<string, TrailParam[]>();
    const objectFiles = revision === 377 ? [unpacked377HerbsFile] : [...TRAIL_TIERS.map((tier) => `${TRAIL_CONFIG_DIR}/trail_${tier}.obj`), TRAIL_CASKET_RELATIVE];
    for (const relative of objectFiles) {
        for (const [alias, params] of parseTrailObjBlocks(requireGatherText(content, relative))) {
            if (blocks.has(alias)) throw new Error(`${alias}: declared by more than one trail obj file`);
            blocks.set(alias, params);
        }
    }
    const rows: TrailMembershipRow[] = [];
    const namedCaskets: string[] = [];
    for (const tier of TRAIL_TIERS) {
        const enumRelative = `${TRAIL_CONFIG_DIR}/trail_${tier}.enum`;
        for (const alias of parseTrailEnumAliases(requireGatherText(content, enumRelative))) {
            if (!blocks.has(alias)) throw new Error(`${enumRelative}: ${alias} has no obj block`);
            const joined = trailJoin(objPack, itemIds, alias, enumRelative);
            const params = blocks.get(alias) as TrailParam[];
            rows.push({ alias: joined.alias, id: joined.id, role: 'clue', params });
            for (const param of params) {
                if (param.key === 'trail_casket' && !namedCaskets.includes(param.value)) namedCaskets.push(param.value);
            }
        }
    }
    for (const alias of namedCaskets) {
        const joined = trailJoin(objPack, itemIds, alias, 'trail_casket param');
        rows.push({ alias: joined.alias, id: joined.id, role: 'casket', params: blocks.get(alias) ?? [] });
    }
    if (revision !== 377) {
        const constrained = rows.filter((row) => row.alias === TRAIL_CONSTRAINED.alias);
        if (constrained.length !== 1) throw new Error(`trails: ${TRAIL_CONSTRAINED.alias} must be one membership row`);
        constrained[0]!.access = 'constrained';
    }
    const challenge_answers: TrailChallengeAnswer[] = [];
    for (const [alias, params] of blocks) {
        const answers = params.filter((param) => param.key === 'trail_challenge_answer');
        if (answers.length === 0) continue;
        if (revision !== 377 && answers.length !== 1) throw new Error(`${alias}: repeated trail_challenge_answer`);
        const joined = trailJoin(objPack, itemIds, alias, 'trail_challenge_answer');
        let answer = answers[0].value;
        if (revision === 377) {
            const decoded = itemIds.get(alias)?.params?.get(answerParamId!);
            const selected = typeof decoded === 'string' ? decoded : typeof decoded === 'number' && Number.isInteger(decoded) ? String(decoded) : undefined;
            if (selected === undefined || !answers.some(param => param.value === selected)) {
                throw new Error(`${alias}: decoded trail_challenge_answer does not match selected source params`);
            }
            answer = selected;
        }
        challenge_answers.push({ alias: joined.alias, id: joined.id, answer });
    }
    return { rows, challenge_answers };
}


/* ------------------------------------------------------------------------- *
 * talk_key: the selected talk steps and key keepers
 *
 * A sibling family of `trails`, never a field on a membership row. Talk steps
 * are the membership clues an `opnpc1` handler checks; key keepers are the
 * `trail_checkmediumdrop` arms. Spawns are the static `==== NPC ====` jm2 rows:
 * one hit publishes a tile, no hit throws for a packed identity, two or more
 * omit the spawn and record honest coverage. Unique spawn and 0-hit throws are
 * the only two outcomes; first-in-file is never taken.
 * ------------------------------------------------------------------------- */

const TALK_KEY_SCRIPTS_DIRECTORY = 'scripts';
const TALK_KEY_NPC_PACK_RELATIVE = 'pack/npc.pack';
const TALK_KEY_MEDIUM_RELATIVE = 'scripts/minigames/game_trail/scripts/medium/trail_clue_medium.rs2';
const TALK_KEY_RS2_SUFFIX = '.rs2';
const TALK_KEY_NPC_SUFFIX = '.npc';

/** One jm2 `==== NPC ====` spawn in world coordinates. Plane, never scene `level`. */
export type TalkKeySpawn = { x: number; z: number; plane: number };

/**
 * Keeper matcher, discriminated on `kind`. `type` is one packed npc id and carries
 * the `.npc` display name; `category` and `name` match many npcs and never carry
 * an alias or an id.
 */
export type TalkKeyKeeper =
    | { kind: 'type'; alias: string; id: number; name: string }
    | { kind: 'category'; category: string }
    | { kind: 'name'; name: string };

/** One opnpc1 talk step: the membership clue that anchors it and the NPC it names. */
export type TalkKeyTalkRow = { alias: string; id: number; npc: { alias: string; id: number; name: string }; spawn?: TalkKeySpawn };

/** One key-keeper step: the membership clue, its key object, and the keeper matcher. */
export type TalkKeyKeyRow = { alias: string; id: number; key_alias: string; key_id: number; keeper: TalkKeyKeeper; spawn?: TalkKeySpawn };

/** Why one step publishes no spawn. A coverage record is a sibling, not a row. */
export type TalkKeyCoverageRow = { class: string; family: string; alias: string; reason: string };

/** Talk steps and key keepers. A step without a unique spawn keeps its row and omits `spawn`. */
export type TalkKeyFacts = { talk: TalkKeyTalkRow[]; keys: TalkKeyKeyRow[]; coverage: TalkKeyCoverageRow[] };


/** One keeper matcher as written in the proc, before the pack and `.npc` joins. */
type TalkKeyKeeperMatch = { kind: 'type'; alias: string } | { kind: 'category'; category: string } | { kind: 'name'; name: string };

/**
 * NPC placements only — the `==== NPC ====` section of one jm2, mirroring the LOC
 * parser's fail-closed gating on a different section. Rows are `plane lx lz: npc_id`
 * with exactly one data token: `==== LOC ====` and `==== OBJ ====` are not placements,
 * and a shape, angle, or any other extra token throws.
 */
export function parseJm2NpcPlacements(text: string) {
    const placements: { plane: number; lx: number; lz: number; npc_id: number }[] = [];
    let inNpc = false;
    for (const raw of text.split(/\r?\n/)) {
        const line = raw.trim();
        if (!line) continue;
        const section = jm2SectionName(line);
        if (section !== null) {
            inNpc = section === 'NPC';
            continue;
        }
        if (!inNpc) continue;
        const colon = line.indexOf(':');
        if (colon <= 0) throw new Error(`jm2 NPC: malformed row ${line}`);
        const coordTokens = line.slice(0, colon).trim().split(/\s+/);
        if (coordTokens.length !== 3) throw new Error(`jm2 NPC: bad coords ${line}`);
        const plane = integer(coordTokens[0], line);
        const lx = integer(coordTokens[1], line);
        const lz = integer(coordTokens[2], line);
        if (plane < 0 || plane > 3) throw new Error(`jm2 NPC: plane out of range ${line}`);
        if (lx < 0 || lx > 63 || lz < 0 || lz > 63) throw new Error(`jm2 NPC: local coords out of range ${line}`);
        const dataTokens = line.slice(colon + 1).trim().split(/\s+/).filter((token) => token.length > 0);
        if (dataTokens.length === 0) throw new Error(`jm2 NPC: missing npc id ${line}`);
        if (dataTokens.length > 1) throw new Error(`jm2 NPC: extra tokens ${line}`);
        placements.push({ plane, lx, lz, npc_id: integer(dataTokens[0], line) });
    }
    return placements;
}

/**
 * `opnpc1` handlers that check one of the three membership enums, in file order.
 * Talk-to only: `opnpc2`, `opnpc3`, `opnpcu`, and `apnpc*` are never read. The
 * identity is the header alias, except on a `_<category>` header, which must
 * declare exactly one inner `npc_type = <alias>` — the category itself is a
 * trigger, not an NPC. Clues outside the membership set (challenge scrolls,
 * puzzle-box extras) are dropped.
 */
export function parseTalkKeyHandlers(text: string, membership: ReadonlySet<string>) {
    const rows: { handler: string; alias: string; clue: string }[] = [];
    let header: string | null = null;
    let body: string[] = [];
    const flush = () => {
        if (header === null) return;
        const parts = header.split(',').map((part) => part.trim());
        const handler = parts[1] ?? '';
        if (parts[0] === 'opnpc1' && handler) {
            const block = body.join('\n');
            const clues = [...new Set([...block.matchAll(/inv_total\(\s*inv\s*,\s*(trail_clue_[A-Za-z0-9_]+)\s*\)/g)].map((match) => match[1]))].filter((clue) => membership.has(clue)).sort();
            if (clues.length > 0) {
                let alias = handler;
                if (handler.startsWith('_')) {
                    const inner = [...new Set([...block.matchAll(/npc_type\s*=\s*([A-Za-z0-9_]+)/g)].map((match) => match[1]))];
                    if (inner.length !== 1) throw new Error(`talk_key: ${header} must declare exactly one inner npc_type, got ${inner.length}`);
                    alias = inner[0];
                }
                for (const clue of clues) rows.push({ handler, alias, clue });
            }
        }
        header = null;
        body = [];
    };
    for (const raw of text.split(/\r?\n/)) {
        const line = raw.trim();
        if (line.startsWith('[') && line.endsWith(']')) {
            flush();
            header = line.slice(1, -1);
            continue;
        }
        if (header !== null) body.push(line);
    }
    flush();
    return rows;
}

/**
 * The `[proc,trail_checkmediumdrop]` arms, in file order. Each arm is one keeper
 * matcher, one membership clue check, and one `obj_add(npc_coord, key, ...)` whose
 * `obj_gettotal` guard names the same key; a loc-search label is not an arm and a
 * malformed arm throws. Call sites of the proc are not a second mapping.
 */
export function parseTalkKeyKeeperArms(text: string) {
    let body: string | null = null;
    let header: string | null = null;
    let lines: string[] = [];
    const flush = () => {
        if (header === 'proc,trail_checkmediumdrop') body = lines.join('\n');
        header = null;
        lines = [];
    };
    for (const raw of text.split(/\r?\n/)) {
        const line = raw.trim();
        if (line.startsWith('[') && line.endsWith(']')) {
            flush();
            header = line.slice(1, -1);
            continue;
        }
        if (header !== null) lines.push(line);
    }
    flush();
    if (body === null) throw new Error('talk_key: required content is missing [proc,trail_checkmediumdrop]');
    const arms: { clue: string; key: string; keeper: TalkKeyKeeperMatch }[] = [];
    for (const arm of body.split(/else\s+if\(/)) {
        const types = [...new Set([...arm.matchAll(/npc_type\s*=\s*([A-Za-z0-9_]+)/g)].map((match) => match[1]))];
        const categories = [...new Set([...arm.matchAll(/npc_category\s*=\s*([A-Za-z0-9_]+)/g)].map((match) => match[1]))];
        const names = [...new Set([...arm.matchAll(/compare\(\s*npc_name\s*,\s*"([^"]+)"\s*\)\s*=\s*0/g)].map((match) => match[1]))];
        const clues = [...new Set([...arm.matchAll(/inv_total\(\s*inv\s*,\s*(trail_clue_[A-Za-z0-9_]+)\s*\)\s*>\s*0/g)].map((match) => match[1]))];
        const added = [...new Set([...arm.matchAll(/obj_add\(\s*npc_coord\s*,\s*([A-Za-z0-9_]+)/g)].map((match) => match[1]))];
        if (clues.length === 0 && types.length === 0 && categories.length === 0 && names.length === 0 && added.length === 0) continue;
        const matches = types.length + categories.length + names.length;
        if (matches !== 1 || clues.length !== 1 || added.length !== 1) throw new Error(`talk_key: malformed trail_checkmediumdrop arm (${matches} keeper matches, ${clues.length} clues, ${added.length} keys)`);
        const guards = [...new Set([...arm.matchAll(/obj_gettotal\(\s*([A-Za-z0-9_]+)\s*\)\s*=\s*0/g)].map((match) => match[1]))];
        if (guards.length !== 1 || guards[0] !== added[0]) throw new Error(`talk_key: trail_checkmediumdrop arm for ${clues[0]} must guard the key it adds`);
        const keeper: TalkKeyKeeperMatch = types.length === 1 ? { kind: 'type', alias: types[0] } : categories.length === 1 ? { kind: 'category', category: categories[0] } : { kind: 'name', name: names[0] };
        arms.push({ clue: clues[0], key: added[0], keeper });
    }
    if (arms.length === 0) throw new Error('talk_key: trail_checkmediumdrop has no keeper arms');
    return arms;
}

/** Required content files discovered by extension in the selected local tree. */
function contentTreeFiles(content: string, directory: string, suffix: string): string[] {
    const rootDirectory = path.join(content, directory);
    if (!fs.existsSync(rootDirectory)) throw new Error(`${directory}: required directory missing`);
    const files: string[] = [];
    const walk = (absolute: string) => {
        for (const entry of fs.readdirSync(absolute, { withFileTypes: true })) {
            const child = path.join(absolute, entry.name);
            if (entry.isDirectory()) walk(child);
            else if (entry.name.endsWith(suffix)) files.push(path.relative(content, child).split(path.sep).join('/'));
        }
    };
    walk(rootDirectory);
    if (files.length === 0) throw new Error(`${directory}: required directory has no ${suffix} files`);
    return files.sort();
}
/**
 * The selected talk steps and key keepers, from the `opnpc1` membership anchors and
 * the medium clue proc. `.npc` sections are read with `parseNpcSection`, so a missing
 * section throws; a section without `name=` throws here. Clue and key aliases join
 * `pack/obj.pack` and the decoded item table through `trailJoin`. Identity names come
 * from the `.npc` configs and are corroborated against the decoded `npc.dat` rows by
 * `assertTalkKeyNpcJoins` in the writer.
 */
export function extractTalkKeyFacts(content: string, items: ObjType[]): TalkKeyFacts {
    const itemIds = new Map(items.filter((item) => item.debugname !== null).map((item) => [item.debugname as string, { id: item.id, name: item.name }]));
    const objPack = parsePack(requireGatherText(content, 'pack/obj.pack'));
    if (objPack.size === 0) throw new Error('pack/obj.pack: required file missing ids');
    const npcPack = parsePack(requireGatherText(content, TALK_KEY_NPC_PACK_RELATIVE));
    if (npcPack.size === 0) throw new Error(`${TALK_KEY_NPC_PACK_RELATIVE}: required file missing ids`);
    const membership = new Set(TRAIL_TIERS.flatMap((tier) => parseTrailEnumAliases(requireGatherText(content, `${TRAIL_CONFIG_DIR}/trail_${tier}.enum`))));
    const scripts = contentTreeFiles(content, TALK_KEY_SCRIPTS_DIRECTORY, TALK_KEY_RS2_SUFFIX);
    const npcConfigs = contentTreeFiles(content, TALK_KEY_SCRIPTS_DIRECTORY, TALK_KEY_NPC_SUFFIX);
    const configText = new Map<string, string>();
    const configSection = (relative: string, alias: string, label: string) => {
        const text = configText.get(relative) ?? fs.readFileSync(path.join(content, relative), 'utf8');
        configText.set(relative, text);
        try {
            return parseNpcSection(text, alias);
        } catch (error) {
            throw new Error(`${label}: ${(error as Error).message}`);
        }
    };
    const configOwners = new Map<string, string>();
    for (const relative of npcConfigs) {
        const text = fs.readFileSync(path.join(content, relative), 'utf8');
        configText.set(relative, text);
        for (const raw of text.split(/\r?\n/)) {
            const line = raw.trim();
            if (!line.startsWith('[') || !line.endsWith(']')) continue;
            const alias = line.slice(1, -1);
            if (configOwners.has(alias)) throw new Error(`npc config: [${alias}] is declared by more than one .npc file`);
            configOwners.set(alias, relative);
        }
    }
    const identity = (alias: string, label: string) => {
        const id = npcPack.get(alias);
        if (id === undefined) throw new Error(`${label}: pack/npc.pack lacks ${alias}`);
        const relative = configOwners.get(alias);
        if (relative === undefined) throw new Error(`${label}: missing [${alias}] in the scanned npc configs`);
        const name = configSection(relative, alias, label).name;
        if (!name) throw new Error(`${label}: [${alias}] has no name`);
        return { alias, id, name };
    };
    const maps = placementMapFiles(content);
    const spawns = new Map<number, TalkKeySpawn[]>();
    for (const map of maps) {
        const { mx, mz } = parseMapsquarePath(map);
        for (const placement of parseJm2NpcPlacements(fs.readFileSync(path.join(content, map), 'utf8'))) {
            const world = worldFromMapsquare(mx, mz, placement.lx, placement.lz, placement.plane);
            const found = spawns.get(placement.npc_id);
            if (found) found.push(world);
            else spawns.set(placement.npc_id, [world]);
        }
    }
    const talk: TalkKeyTalkRow[] = [];
    const keys: TalkKeyKeyRow[] = [];
    const coverage: TalkKeyCoverageRow[] = [];
    const uniqueSpawn = (npcId: number, clue: string, family: 'talk' | 'keys') => {
        const found = spawns.get(npcId) ?? [];
        if (found.length === 1) return found[0];
        if (found.length === 0) throw new Error(`talk_key: ${clue} npc ${npcId} has no jm2 NPC spawn`);
        coverage.push({ class: 'unknown', family, alias: clue, reason: 'non-unique jm2 NPC spawn' });
        return null;
    };
    for (const relative of scripts) {
        for (const handler of parseTalkKeyHandlers(fs.readFileSync(path.join(content, relative), 'utf8'), membership)) {
            if (talk.some((row) => row.alias === handler.clue)) throw new Error(`talk_key: ${handler.clue} is anchored by more than one opnpc1 handler`);
            const npc = identity(handler.alias, `talk_key ${handler.clue}`);
            const joined = trailJoin(objPack, itemIds, handler.clue, `talk_key ${handler.clue}`);
            const spawn = uniqueSpawn(npc.id, handler.clue, 'talk');
            talk.push({ alias: joined.alias, id: joined.id, npc, ...(spawn ? { spawn } : {}) });
        }
    }
    for (const arm of parseTalkKeyKeeperArms(requireGatherText(content, TALK_KEY_MEDIUM_RELATIVE))) {
        if (!membership.has(arm.clue)) throw new Error(`talk_key: keeper clue ${arm.clue} is not a membership alias`);
        const joined = trailJoin(objPack, itemIds, arm.clue, `talk_key ${arm.clue}`);
        const key = trailJoin(objPack, itemIds, arm.key, `talk_key ${arm.clue} key`);
        let keeper: TalkKeyKeeper;
        let spawn: TalkKeySpawn | null = null;
        if (arm.keeper.kind === 'type') {
            const npc = identity(arm.keeper.alias, `talk_key ${arm.clue} keeper`);
            keeper = { kind: 'type', alias: npc.alias, id: npc.id, name: npc.name };
            spawn = uniqueSpawn(npc.id, arm.clue, 'keys');
        } else if (arm.keeper.kind === 'category') {
            keeper = { kind: 'category', category: arm.keeper.category };
            coverage.push({ class: 'unknown', family: 'keys', alias: arm.clue, reason: 'keeper is a category, not one packed npc id' });
        } else {
            keeper = { kind: 'name', name: arm.keeper.name };
            coverage.push({ class: 'unknown', family: 'keys', alias: arm.clue, reason: 'keeper is a name match, not one packed npc id' });
        }
        keys.push({ alias: joined.alias, id: joined.id, key_alias: key.alias, key_id: key.id, keeper, ...(spawn ? { spawn } : {}) });
    }
    talk.sort((a, b) => a.alias.localeCompare(b.alias));
    keys.sort((a, b) => a.alias.localeCompare(b.alias));
    coverage.sort((a, b) => a.family.localeCompare(b.family) || a.alias.localeCompare(b.alias));
    return { talk, keys, coverage };
}

/**
 * Corroborate every published npc identity against the decoded `npc.dat` rows: the
 * packed alias, the packed id, and the `.npc` display name must all agree with the
 * decoder. A disagreement throws instead of being published under the selected name.
 */
export function assertTalkKeyNpcJoins(facts: TalkKeyFacts, npcs: NpcType[]) {
    const decoded = new Map(npcs.filter((npc) => npc.debugname != null).map((npc) => [npc.debugname as string, npc]));
    const check = (alias: string, id: number, name: string, label: string) => {
        const npc = decoded.get(alias);
        if (!npc) throw new Error(`${label}: npc.dat lacks ${alias}`);
        if (npc.id !== id) throw new Error(`${label}: npc.pack ${alias}=${id} disagrees with decoded npc id ${npc.id}`);
        if (npc.name !== name) throw new Error(`${label}: ${alias} display name ${name} disagrees with decoded npc name ${npc.name}`);
    };
    for (const row of facts.talk) check(row.npc.alias, row.npc.id, row.npc.name, row.alias);
    for (const row of facts.keys) if (row.keeper.kind === 'type') check(row.keeper.alias, row.keeper.id, row.keeper.name, row.alias);
}


/* ------------------------------------------------------------------------- *
 * trio_givers: the three selected givers behind the coordinate-tool trio
 *
 * A sibling family of `talk_key`, never a field on a membership row. The
 * identity set is closed: one exact handler path per alias, joined to
 * `pack/npc.pack`, the owning `.npc` name, and the unique jm2 `==== NPC ====`
 * world tile. A lookalike alias (`observatory_professor2`, the `murphy_*`
 * variants, the quest_cog labels) is never published: a display name is not an
 * identity. Two or more spawn hits keep the row and record coverage; zero hits
 * throw for a packed identity, because zero is not an unknown tile.
 * ------------------------------------------------------------------------- */

const TRIO_GIVERS_NPC_PACK_RELATIVE = 'pack/npc.pack';
const TRIO_GIVERS_FAMILY = 'trio_givers';

/**
 * The closed giver set: one alias, the exact script that declares its `opnpc1`
 * header, and the `.npc` config that owns its display name. Required content
 * lists the three script names; that listing is inventory evidence, not this join.
 */
const TRIO_GIVERS: { alias: string; handler: string; config: string }[] = [
    { alias: 'observatory_professor', handler: 'scripts/quests/quest_itgronigen/scripts/observatory_professor.rs2', config: 'scripts/quests/quest_itgronigen/configs/quest_itgronigen.npc' },
    { alias: 'murphy', handler: 'scripts/minigames/game_trawler/scripts/murphy.rs2', config: 'scripts/minigames/game_trawler/configs/trawler.npc' },
    { alias: 'brother_kojo', handler: 'scripts/areas/area_ardougne_east/scripts/brother_kojo.rs2', config: 'scripts/areas/area_ardougne_east/configs/ardougne_east.npc' },
];

/** One jm2 `==== NPC ====` spawn in world coordinates. Plane, never scene `level`. */
export type TrioGiverSpawn = { x: number; z: number; plane: number };

/** One selected giver: the packed NPC identity itself, plus its unique world tile. */
export type TrioGiverRow = { alias: string; id: number; name: string; spawn?: TrioGiverSpawn };

/** Why one giver publishes no spawn. A coverage record is a sibling, not a row. */
export type TrioGiverCoverageRow = { class: string; family: string; alias: string; reason: string };

/** The three selected givers. A giver without a unique spawn keeps its row and omits `spawn`. */
export type TrioGiverFacts = { rows: TrioGiverRow[]; coverage: TrioGiverCoverageRow[] };


/**
 * The `opnpc1` header aliases one exact giver script declares, in file order.
 * `opnpc2`, `opnpc3`, `opnpcu`, and `apnpc*` are never read, and an inner
 * `npc_type` is not an identity: the header alias is. A malformed header throws
 * instead of being skipped.
 */
export function parseTrioGiverHandlers(text: string) {
    const aliases: string[] = [];
    for (const raw of text.split(/\r?\n/)) {
        const line = raw.trim();
        if (!line.startsWith('[') || !line.endsWith(']')) continue;
        const parts = line.slice(1, -1).split(',').map((part) => part.trim());
        if (parts[0] !== 'opnpc1') continue;
        if (parts.length !== 2 || !parts[1]) throw new Error(`trio_givers: malformed opnpc1 header ${line}`);
        aliases.push(parts[1]);
    }
    return aliases;
}

/**
 * The three selected givers, from the closed handler set. Each exact script must
 * declare its `[opnpc1,<alias>]` header, and the alias joins `pack/npc.pack` and
 * the selected revision's NPC source (`.npc` configs, or revision 377's
 * unpacked `all.npc`), so a missing section or name throws. The spawn is the
 * unique jm2 `==== NPC ====` world tile.
 */
export function extractTrioGiversFacts(content: string, revision = 289): TrioGiverFacts {
    const npcPack = parsePack(requireGatherText(content, TRIO_GIVERS_NPC_PACK_RELATIVE));
    if (npcPack.size === 0) throw new Error(`${TRIO_GIVERS_NPC_PACK_RELATIVE}: required file missing ids`);
    const maps = placementMapFiles(content);
    const spawns = new Map<number, TrioGiverSpawn[]>();
    for (const map of maps) {
        const { mx, mz } = parseMapsquarePath(map);
        for (const placement of parseJm2NpcPlacements(fs.readFileSync(path.join(content, map), 'utf8'))) {
            const world = worldFromMapsquare(mx, mz, placement.lx, placement.lz, placement.plane);
            const found = spawns.get(placement.npc_id);
            if (found) found.push(world);
            else spawns.set(placement.npc_id, [world]);
        }
    }
    const rows: TrioGiverRow[] = [];
    const coverage: TrioGiverCoverageRow[] = [];
    for (const giver of TRIO_GIVERS) {
        const aliases = parseTrioGiverHandlers(requireGatherText(content, giver.handler));
        if (!aliases.includes(giver.alias)) throw new Error(`trio_givers: ${giver.handler} does not declare [opnpc1,${giver.alias}]`);
        const crossed = aliases.find((alias) => alias !== giver.alias && TRIO_GIVERS.some((entry) => entry.alias === alias));
        if (crossed) throw new Error(`trio_givers: ${giver.handler} also declares [opnpc1,${crossed}]`);
        const id = npcPack.get(giver.alias);
        if (id === undefined) throw new Error(`trio_givers: ${TRIO_GIVERS_NPC_PACK_RELATIVE} lacks ${giver.alias}`);
        let name: string | undefined;
        const configSource = revision === 377 ? 'scripts/_unpack/377/all.npc' : giver.config;
        try {
            name = parseNpcSection(requireGatherText(content, configSource), giver.alias).name;
        } catch (error) {
            throw new Error(`trio_givers ${giver.alias}: ${(error as Error).message}`);
        }
        if (!name) throw new Error(`trio_givers: [${giver.alias}] has no name`);
        const found = spawns.get(id) ?? [];
        if (found.length === 0) throw new Error(`trio_givers: ${giver.alias} npc ${id} has no jm2 NPC spawn`);
        if (found.length > 1) coverage.push({ class: 'unknown', family: TRIO_GIVERS_FAMILY, alias: giver.alias, reason: 'non-unique jm2 NPC spawn' });
        rows.push({ alias: giver.alias, id, name, ...(found.length === 1 ? { spawn: found[0] } : {}) });

    }
    return { rows, coverage };
}

/**
 * Corroborate every published giver against the decoded `npc.dat` rows: the
 * packed alias, the packed id, and the `.npc` display name must all agree with
 * the decoder. A disagreement throws instead of being published under the
 * selected name.
 */
export function assertTrioGiverNpcJoins(facts: TrioGiverFacts, npcs: NpcType[]) {
    const decoded = new Map(npcs.filter((npc) => npc.debugname != null).map((npc) => [npc.debugname as string, npc]));
    for (const row of facts.rows) {
        const npc = decoded.get(row.alias);
        if (!npc) throw new Error(`trio_givers: npc.dat lacks ${row.alias}`);
        if (npc.id !== row.id) throw new Error(`trio_givers: ${TRIO_GIVERS_NPC_PACK_RELATIVE} ${row.alias}=${row.id} disagrees with decoded npc id ${npc.id}`);
        if (npc.name !== row.name) throw new Error(`trio_givers: ${row.alias} display name ${row.name} disagrees with decoded npc name ${npc.name}`);
    }
}


export function extractFacts(content: string, items: ObjType[], npcs: NpcType[]) {
    const itemIds = new Map(items.filter((item) => item.debugname !== null).map((item) => [item.debugname as string, { id: item.id, name: item.name }]));
    const objectsByAlias = new Map(items.filter((item) => item.debugname !== null).map((item) => [item.debugname as string, item]));
    const aliasesById = new Map(items.filter((item) => item.debugname !== null).map((item) => [item.id, item.debugname as string]));
    const npcIds = new Map(npcs.filter((npc) => npc.debugname != null).map((npc) => [npc.debugname as string, { id: npc.id, name: npc.name, respawn_ticks: npc.respawnrate ?? ENGINE_DEFAULT_NPC_RESPAWN_TICKS }]));
    const categoryNames = new Map([...parsePack(requireGatherText(content, 'pack/category.pack'))].map(([name, id]) => [id, name]));
    const paramIds = parsePack(requireGatherText(content, 'pack/param.pack'));
    const enumNames = new Map([...parsePack(requireGatherText(content, 'pack/enum.pack'))].map(([name, id]) => [id, name]));
    const consumptionEffects = parseConsumptionEffects(requireGatherText(content, 'scripts/player/scripts/consumption/consume.rs2'));
    const messageDelays = parseConsumeMessageDelays(requireGatherText(content, 'scripts/player/configs/consumption/consume_messages.dbrow'));
    const parameter = (object: ObjType, name: string) => {
        const id = paramIds.get(name);
        if (id === undefined) throw new Error(`pack/param.pack: missing ${name}`);
        return object.params?.get(id);
    };
    const paramReference = (value: number | string | undefined, namesById: ReadonlyMap<number, string>, label: string) => {
        if (value === undefined) return null;
        if (typeof value === 'number') {
            if (value < 0) return null;
            const name = namesById.get(value);
            if (!name) throw new Error(`${label}: unknown referenced id ${value}`);
            return name;
        }
        const name = value.trim().replace(/^\^/, '');
        if (!name || name === 'null' || name === 'none' || name === '-1') return null;
        if (/^-?\d+$/.test(name)) {
            const id = Number(name);
            if (id < 0) return null;
            const referenced = namesById.get(id);
            if (!referenced) throw new Error(`${label}: unknown referenced id ${name}`);
            return referenced;
        }
        return name;
    };
    const sourceInfo = (alias: string) => {
        const object = objectsByAlias.get(alias);
        if (!object) throw new Error(`consumption ${alias}: missing selected object config`);
        const category = object.category === undefined || object.category < 0 ? undefined : categoryNames.get(object.category);
        if (object.category !== undefined && object.category >= 0 && category === undefined) {
            throw new Error(`obj ${alias}: category ${object.category} is absent from pack/category.pack`);
        }
        const nextStage = paramReference(parameter(object, 'next_obj_stage'), aliasesById, `obj ${alias} next_obj_stage`);
        const familyName = paramReference(parameter(object, 'decant_potion_enum'), enumNames, `obj ${alias} decant_potion_enum`);
        const rawDoseCount = parameter(object, 'dose_count');
        let doseCount: number | undefined;
        if (rawDoseCount !== undefined) {
            const count = typeof rawDoseCount === 'number' ? rawDoseCount : Number(rawDoseCount);
            if (!Number.isInteger(count)) throw new Error(`obj ${alias}: dose_count is not an integer`);
            doseCount = count;
        }
        if (category === 'potion' && (familyName !== null || doseCount !== undefined)) {
            if (!familyName || doseCount === undefined || doseCount < 1 || doseCount > 4) {
                throw new Error(`obj ${alias}: incomplete potion dose family`);
            }
        }
        return {
            category,
            next_stage: nextStage,
            dose_family: category === 'potion' ? familyName : null,
            dose_count: category === 'potion' ? doseCount ?? null : null,
        };
    };
    type ConsumptionRow = {
        action: 'consume';
        item: { alias: string; id: number; name: string };
        source_row: string;
        source_file: string;
        effect: string;
        eat_delay_arg: number | null;
        skill_delay_arg: number | null;
        message_delay: number | null;
        stat_change: { stat: string; base: number; percent: number }[];
        stat_heal: { stat: string; base: number; percent: number }[];
        heal_energy: number[];
        qualification: string;
        next_stage: string | null;
        dose_family: string | null;
        dose_count: number | null;
    };
    const consumption: ConsumptionRow[] = [];
    const consumedAliases = new Set<string>();
    for (const file of ['consume_normal.dbrow', 'consume_effects.dbrow']) {
        for (const parsed of parseRows(fs.readFileSync(path.join(content, 'scripts/player/configs/consumption', file), 'utf8'))) {
            const changes = parsed.values.stat_change ?? [];
            const heals = parsed.values.stat_heal ?? [];
            const energy = parsed.values.healenergy ?? [];
            const statChange = changes.map((value) => ({ stat: value[0], base: integer(value[1], parsed.name), percent: integer(value[2], parsed.name) }));
            const statHeal = heals.map((value) => ({ stat: value[0], base: integer(value[1], parsed.name), percent: integer(value[2], parsed.name) }));
            const fixedHp = file === 'consume_normal.dbrow' && statChange.length === 0 && statHeal.length === 1 && statHeal[0].stat === 'hitpoints' && statHeal[0].percent === 0 && statHeal[0].base > 0 && energy.length === 0;
            for (const [alias] of parsed.values.consumable ?? []) {
                const item = itemIds.get(alias);
                if (!item) throw new Error(`consumption ${file}/${parsed.name}: unknown item ${alias}`);
                if (!item.name) throw new Error(`consumption ${file}/${parsed.name}: ${alias} has no display name`);
                const info = sourceInfo(alias);
                const timing = consumptionRule(consumptionEffects, alias, info.category, info.next_stage);
                const handled = timing !== null;
                consumption.push({
                    action: 'consume',
                    item: { alias, id: item.id, name: item.name },
                    source_row: parsed.name,
                    source_file: file,
                    effect: timing?.effect ?? '',
                    eat_delay_arg: timing?.eat_delay_arg ?? null,
                    skill_delay_arg: timing?.skill_delay_arg ?? null,
                    message_delay: handled ? messageDelays.get(alias) ?? null : null,
                    stat_change: statChange,
                    stat_heal: statHeal,
                    heal_energy: energy.map((value) => integer(value[0], parsed.name)),
                    qualification: fixedHp && handled ? 'fixed_hp_heal' : 'not_fixed_hp_heal',
                    next_stage: info.next_stage,
                    dose_family: info.dose_family,
                    dose_count: info.dose_count,
                });
                consumedAliases.add(alias);
            }
        }
    }
    for (const [alias, item] of itemIds) {
        const info = sourceInfo(alias);
        if (info.category !== 'potion' || !info.dose_family || info.dose_count === null || consumedAliases.has(alias)) continue;
        if (!item.name) throw new Error(`consumption consume.rs2: ${alias} has no display name`);
        const timing = consumptionRule(consumptionEffects, alias, info.category, info.next_stage);
        consumption.push({
            action: 'consume',
            item: { alias, id: item.id, name: item.name },
            source_row: 'consume.rs2',
            source_file: 'consume.rs2',
            effect: timing?.effect ?? '',
            eat_delay_arg: timing?.eat_delay_arg ?? null,
            skill_delay_arg: timing?.skill_delay_arg ?? null,
            message_delay: timing === null ? null : messageDelays.get(alias) ?? null,
            stat_change: [],
            stat_heal: [],
            heal_energy: [],
            qualification: 'not_fixed_hp_heal',
            next_stage: info.next_stage,
            dose_family: info.dose_family,
            dose_count: info.dose_count,
        });
    }
    type PickpocketNpc = { alias: string; id: number; name: string; respawn_ticks: number };
    type PickpocketRow = {
        group: string;
        npcs: PickpocketNpc[];
        level: number;
        experience: number;
        stun_ticks: number;
        stun_damage: number;
        success_chance: { numerator: number; denominator: number };
        loot: { item: PickpocketNpc; min: number; max: number; weight: number }[];
        pocket: string;
    };
    type PickpocketLevelOnlyRow = {
        kind: 'level_only';
        source_file: string;
        target_row: string;
        source_row: string;
        npcs: PickpocketNpc[];
        level: number;
    };
    const pickpocket: (PickpocketRow | PickpocketLevelOnlyRow)[] = [];
    for (const parsed of parseRows(fs.readFileSync(path.join(content, 'scripts/skill_thieving/configs/pickpocking/pickpocket.dbrow'), 'utf8'))) {
        const npc = (parsed.values.npc ?? []).map((value) => {
            const found = npcIds.get(value[0]);
            if (!found) throw new Error(`pickpocket ${parsed.name}: unknown NPC ${value[0]}`);
            if (!found.name) throw new Error(`pickpocket ${parsed.name}: NPC ${value[0]} has no display name`);
            return { alias: value[0], id: found.id, name: found.name, respawn_ticks: found.respawn_ticks };
        });
        const chance = parsed.values.success_chance?.[0];
        if (!chance || chance.length !== 2) throw new Error(`${parsed.name}: malformed success chance`);
        const loot = (parsed.values.loot ?? []).map((value) => {
            const item = itemIds.get(value[0]);
            if (!item) throw new Error(`pickpocket ${parsed.name}: unknown loot ${value[0]}`);
            if (!item.name) throw new Error(`pickpocket ${parsed.name}: loot ${value[0]} has no display name`);
            return {
                item: { alias: value[0], id: item.id, name: item.name },
                min: integer(value[1], parsed.name),
                max: integer(value[2], parsed.name),
                weight: integer(value[3], parsed.name),
            };
        });
        pickpocket.push({
            group: parsed.name,
            npcs: npc,
            level: integer(required(parsed.values, 'level', parsed.name), parsed.name),
            experience: integer(required(parsed.values, 'experience', parsed.name), parsed.name),
            stun_ticks: integer(required(parsed.values, 'stun_ticks', parsed.name), parsed.name),
            stun_damage: integer(required(parsed.values, 'stun_damage', parsed.name), parsed.name),
            success_chance: { numerator: integer(chance[0], parsed.name), denominator: integer(chance[1], parsed.name) },
            loot,
            pocket: required(parsed.values, 'pocket', parsed.name),
        });
    }
    for (const source of thievingLevelOnlySources) {
        const aliases = source.aliases.filter((alias) => npcIds.has(alias));
        if (aliases.length === 0) continue;
        const file = path.join(content, source.file);
        if (!fs.existsSync(file)) throw new Error(`${source.file}: missing selected Thieving level source`);
        const script = fs.readFileSync(file, 'utf8');
        const gates = [...script.matchAll(/^\s*if\s*\(\s*stat\s*\(\s*thieving\s*\)\s*<\s*(\d+)\s*\)\s*\{\s*$/gm)];
        if (gates.length !== 1) throw new Error(`${source.file}: expected one explicit Thieving level gate`);
        const level = integer(gates[0][1], source.file);
        if (level < 1) throw new Error(`${source.file}: invalid Thieving level gate`);
        const targetRows = new Map<string, string[]>();
        for (const match of script.matchAll(/^\s*\[opnpc3,([a-z0-9_]+)\][^\r\n]*$/gm)) {
            const rows = targetRows.get(match[1]) ?? [];
            rows.push(match[0].trim());
            targetRows.set(match[1], rows);
        }
        for (const alias of aliases) {
            const found = npcIds.get(alias);
            if (!found?.name) throw new Error(`${source.file}: ${alias} has no selected NPC name`);
            const rows = targetRows.get(alias);
            if (!rows || rows.length !== 1) throw new Error(`${source.file}: expected one opnpc3 source row for ${alias}`);
            pickpocket.push({
                kind: 'level_only',
                source_file: source.file,
                target_row: rows[0],
                source_row: gates[0][0].trim(),
                npcs: [{ alias, id: found.id, name: found.name, respawn_ticks: found.respawn_ticks }],
                level,
            });
        }
    }
    return { consumption, pickpocket };
}
type CatalogBank = {
    name: string; tile: { x: number; z: number; level: number };
    approach?: { x: number; z: number; level: number };
    requires?: { skill?: { name: string; level: number }; quest?: string; setting?: string };
    access?: { name: string; op: string; openFirst?: { name: string; op: string } };
    npcAccess?: { name: string; op: string; choose?: string };
};

// The runtime-selected engine owns TypeScript. Only fields guarded by its
// is* predicates are read; no foreign module is executed.
type BankAstNode = {
    text: string; name: BankAstNode; initializer: BankAstNode;
    elements: BankAstNode[]; properties: BankAstNode[]; arguments: BankAstNode[];
    expression: BankAstNode; getText(source: unknown): string;
};
type BankLiteral = string | number | BankLiteral[] | { [key: string]: BankLiteral };

function catalogBank(value: BankLiteral): CatalogBank {
    const record = (input: BankLiteral): { [key: string]: BankLiteral } => {
        if (input === null || typeof input !== 'object' || Array.isArray(input)) throw new Error('bank catalog: expected object');
        return input;
    };
    const string = (input: BankLiteral) => {
        if (typeof input !== 'string') throw new Error('bank catalog: expected string');
        return input;
    };
    const integer = (input: BankLiteral) => {
        if (typeof input !== 'number' || !Number.isInteger(input)) throw new Error('bank catalog: expected integer');
        return input;
    };
    const tile = (input: BankLiteral) => {
        const row = record(input);
        return { x: integer(row.x), z: integer(row.z), level: integer(row.level) };
    };
    const operation = (input: BankLiteral) => {
        const row = record(input);
        return { name: string(row.name), op: string(row.op) };
    };
    const row = record(value);
    const bank: CatalogBank = { name: string(row.name), tile: tile(row.tile) };
    if (row.approach !== undefined) bank.approach = tile(row.approach);
    if (row.requires !== undefined) {
        const req = record(row.requires);
        bank.requires = {};
        if (req.quest !== undefined) bank.requires.quest = string(req.quest);
        if (req.setting !== undefined) bank.requires.setting = string(req.setting);
        if (req.skill !== undefined) {
            const skill = record(req.skill);
            bank.requires.skill = { name: string(skill.name), level: integer(skill.level) };
        }
    }
    if (row.access !== undefined) {
        const access = record(row.access);
        bank.access = operation(row.access);
        if (access.openFirst !== undefined) bank.access.openFirst = operation(access.openFirst);
    }
    if (row.npcAccess !== undefined) {
        const npc = record(row.npcAccess);
        bank.npcAccess = operation(row.npcAccess);
        if (npc.choose !== undefined) bank.npcAccess.choose = string(npc.choose);
    }
    return bank;
}

/**
 * Named top-level literals from an rs2b0t source, read from its AST:
 * strings, numbers, arrays, object literals, `new Tile(x, z, level)` and
 * references to other top-level constants. Nothing is evaluated.
 */
async function topLevelLiteralReader(engine: string, fileName: string, source: string, label: string) {
    // Engine path is selected per revision; a static import would bind the wrong installation.
    const ts = await import(pathToFileURL(path.join(engine, 'node_modules/typescript/lib/typescript.js')).href);
    const ast = ts.createSourceFile(fileName, source, ts.ScriptTarget.Latest, true);
    const constants = new Map<string, BankAstNode>();
    for (const statement of ast.statements) {
        if (ts.isVariableStatement(statement)) {
            for (const declaration of statement.declarationList.declarations) {
                if (ts.isIdentifier(declaration.name)) constants.set(declaration.name.text, declaration.initializer);
            }
        }
    }
    function literal(node: BankAstNode | undefined): BankLiteral {
        if (!node) throw new Error(`${label}: missing literal`);
        if (ts.isStringLiteral(node)) return node.text;
        if (ts.isNumericLiteral(node)) return Number(node.text);
        if (ts.isIdentifier(node) && constants.has(node.text)) return literal(constants.get(node.text));
        if (ts.isArrayLiteralExpression(node)) return node.elements.map(literal);
        if (ts.isObjectLiteralExpression(node)) {
            return Object.fromEntries(node.properties.map((property: BankAstNode) => {
                if (!ts.isPropertyAssignment(property)) throw new Error(`${label}: nonliteral property`);
                return [property.name.text, literal(property.initializer)];
            }));
        }
        if (ts.isNewExpression(node) && node.expression.getText(ast) === 'Tile') {
            const [x, z, level] = node.arguments.map(literal);
            if (![x, z, level].every(value => typeof value === 'number' && Number.isInteger(value))) throw new Error(`${label}: invalid tile`);
            return { x, z, level };
        }
        throw new Error(`${label}: unsupported AST ${node.getText(ast)}`);
    }
    return (name: string) => literal(constants.get(name));
}

/** Read the top-level literal AST, not a second curated copy or evaluated foreign module. */
export async function extractBankCatalog(engine: string, source: string) {
    const literal = await topLevelLiteralReader(engine, 'BankLocations.ts', source, 'bank catalog');
    const value = literal('BANK_LOCATIONS');
    if (!Array.isArray(value)) throw new Error('bank catalog: expected array');
    const rows = value.map(catalogBank);
    if (!rows.length || new Set(rows.map(row => row.name)).size !== rows.length) throw new Error('bank catalog: empty or duplicate names');
    return rows;
}

type CookTile = { x: number; z: number; level: number };
type CookKind = 'oven' | 'fire';
/** A curated bank cook surface (`curatedPlan`, `data/cookLocations.ts:98-116`). */
type CookCamp = { bank: string; stand: CookTile; approach?: CookTile; loc: CookTile; locName: string; kind: CookKind; label: string };
export type CookCatalog = { camps: CookCamp[]; kinds: Record<string, CookKind>; maxSurfaceCheb: number; derivedArriveRadius: number; obstacles: string[] };

/**
 * The cook pairing inputs, read from the local AST: the curated bank
 * surfaces (`CURATED_CAMP` → `FISH_CAMP_COOK_PLANS[camp].bank ?? .pier`,
 * `data/cookLocations.ts:92-116`, `data/cookingRanges.ts:98-181`), the
 * surface loc kinds (`tools/cooking/gen-cooksurfaces.ts` `COOK_SURFACE_KINDS`)
 * and the pairing constants (`MAX_SURFACE_CHEB`, `DERIVED_ARRIVE_RADIUS`,
 * `DEFAULT_OBSTACLES`).
 */
export async function extractCookCatalog(engine: string, sources: { cookLocations: string; cookingRanges: string; genCookSurfaces: string }): Promise<CookCatalog> {
    const label = 'cook catalog';
    const record = (input: BankLiteral): { [key: string]: BankLiteral } => {
        if (input === null || typeof input !== 'object' || Array.isArray(input)) throw new Error(`${label}: expected object`);
        return input;
    };
    const string = (input: BankLiteral) => {
        if (typeof input !== 'string') throw new Error(`${label}: expected string`);
        return input;
    };
    const integer = (input: BankLiteral) => {
        if (typeof input !== 'number' || !Number.isInteger(input)) throw new Error(`${label}: expected integer`);
        return input;
    };
    const tile = (input: BankLiteral): CookTile => {
        const row = record(input);
        return { x: integer(row.x), z: integer(row.z), level: integer(row.level) };
    };
    const locations = await topLevelLiteralReader(engine, 'cookLocations.ts', sources.cookLocations, label);
    const ranges = await topLevelLiteralReader(engine, 'cookingRanges.ts', sources.cookingRanges, label);
    const generator = await topLevelLiteralReader(engine, 'gen-cooksurfaces.ts', sources.genCookSurfaces, label);
    const plans = record(ranges('FISH_CAMP_COOK_PLANS'));
    const camps = Object.entries(record(locations('CURATED_CAMP'))).map(([bank, campName]) => {
        // `cookSurfaceForFishCamp(camp, 'bank')`: the bank surface, else the pier one (`cookingRanges.ts:169-181`).
        const plan = record(plans[string(campName)] ?? {});
        const chosen = plan.bank ?? plan.pier;
        if (chosen === undefined) throw new Error(`${label}: no plan for ${bank}`);
        const surface = record(chosen);
        const stand = tile(surface.stand);
        const locName = string(surface.locName);
        const camp: CookCamp = {
            bank,
            stand,
            loc: surface.loc === undefined ? stand : tile(surface.loc),
            locName,
            // `curatedPlan`: a curated `range` is an oven, anything else a fire (`cookLocations.ts:111`).
            kind: string(surface.kind) === 'range' ? 'oven' : 'fire',
            label: surface.label === undefined ? locName : string(surface.label),
        };
        if (surface.approach !== undefined) camp.approach = tile(surface.approach);
        return camp;
    });
    const kinds = Object.fromEntries(Object.entries(record(generator('COOK_SURFACE_KINDS'))).map(([debugname, kind]) => {
        if (kind !== 'oven' && kind !== 'fire') throw new Error(`${label}: unknown kind ${String(kind)}`);
        return [debugname, kind as CookKind];
    }));
    const obstacles = locations('DEFAULT_OBSTACLES');
    if (!Array.isArray(obstacles)) throw new Error(`${label}: expected obstacle array`);
    return {
        camps,
        kinds,
        maxSurfaceCheb: integer(locations('MAX_SURFACE_CHEB')),
        derivedArriveRadius: integer(locations('DERIVED_ARRIVE_RADIUS')),
        obstacles: obstacles.map(string),
    };
}

export function cookCatalogJson(catalog: CookCatalog) {
    const facts = {
        max_surface_cheb: catalog.maxSurfaceCheb,
        derived_arrive_radius: catalog.derivedArriveRadius,
        default_obstacles: catalog.obstacles,
        camps: catalog.camps.map(camp => ({
            bank: camp.bank,
            stand: camp.stand,
            approach: camp.approach ?? null,
            loc: camp.loc,
            loc_name: camp.locName,
            kind: camp.kind,
            label: camp.label,
        })),
    };
    return `${JSON.stringify({ schema: 1, facts }, null, 2)}\n`;
}

export function bankCatalogJson(rows: CatalogBank[]) {
    const facts = rows.map(row => {
        const skill = row.requires?.skill;
        // The frozen catalog currently has one skill gate; refuse silently assigning another id.
        if (skill && skill.name !== 'fishing') throw new Error(`bank catalog: unknown skill ${skill.name}`);
        return {
            name: row.name,
            tile: row.tile,
            approach: row.approach ?? null,
            skill: skill ? [10, skill.level] : null,
            quest: row.requires?.quest ?? null,
            setting: row.requires?.setting ?? null,
            object: row.access ? { name: row.access.name, op: row.access.op } : null,
            open_first: row.access?.openFirst ? { name: row.access.openFirst.name, op: row.access.openFirst.op } : null,
            npc: row.npcAccess ? { name: row.npcAccess.name, op: row.npcAccess.op } : null,
            choose: row.npcAccess?.choose ?? null,
        };
    });
    return `${JSON.stringify({ schema: 1, facts }, null, 2)}\n`;
}

/** The level-1 MAP tiles whose flags carry LINK_BELOW (0x2), as `lx,lz`. */
export function parseJm2LinkBelow(text: string) {
    const tiles = new Set<string>();
    let inMap = false;
    for (const raw of text.split(/\r?\n/)) {
        const line = raw.trim();
        if (!line) continue;
        const section = jm2SectionName(line);
        if (section !== null) {
            inMap = section === 'MAP';
            continue;
        }
        if (!inMap) continue;
        const colon = line.indexOf(':');
        if (colon <= 0) throw new Error(`jm2 MAP: malformed row ${line}`);
        const [plane, lx, lz] = line.slice(0, colon).trim().split(/\s+/).map((token) => integer(token, line));
        if (plane !== 1) continue;
        const flags = line.slice(colon + 1).trim().split(/\s+/).find((token) => token.startsWith('f'));
        if (flags !== undefined && (integer(flags.slice(1), line) & 0x2) !== 0) tiles.add(`${lx},${lz}`);
    }
    return tiles;
}

/**
 * Cook surfaces placed in the selected map pack. Legacy revisions retain the
 * frozen kind queries and camp checks. Revision 377 derives kinds from the
 * selected cooking categories and does not inherit curated camp coordinates.
 */
export function extractCookSurfaces(content: string, catalog: CookCatalog | undefined, revision = 289) {
    const locFiles = contentTreeFiles(content, 'scripts', '.loc');
    const maps = placementMapFiles(content);
    const packed = parsePack(fs.readFileSync(path.join(content, 'pack/loc.pack'), 'utf8'));
    if (revision !== 377 && !catalog) throw new Error('cook surfaces: legacy catalog is required');
    const kinds: Record<string, CookKind> = revision === 377 ? {} : catalog!.kinds;
    if (revision === 377) {
        const handlerPath = 'scripts/skill_cooking/scripts/cooking.rs2';
        const text = fs.readFileSync(path.join(content, handlerPath), 'utf8').replace(/\/\/[^\n]*/g, '');
        for (const category of ['cooking_oven', 'cooking_fire']) {
            const section = text.split(/(?=^\[)/m).find(block => block.startsWith(`[oplocu,_${category}]`));
            if (!section?.includes('@attempt_cook_item(last_useitem)')) throw new Error(`cook surfaces: ${category} cooking handler is not derived`);
        }
    }
    const names = new Map<string, string>();
    for (const file of locFiles) {
        for (const [alias, config] of parseNpcConfigSections(fs.readFileSync(path.join(content, file), 'utf8'))) {
            if (revision === 377) {
                if (config.category === 'cooking_oven') kinds[alias] = 'oven';
                else if (config.category === 'cooking_fire') kinds[alias] = 'fire';
            }
            if (kinds[alias] !== undefined && config.name !== undefined) names.set(alias, config.name);
        }
    }
    const wanted = new Map<number, { debugname: string; name: string; kind: CookKind }>();
    for (const [alias, id] of packed) {
        const kind = kinds[alias];
        if (kind !== undefined) wanted.set(id, { debugname: alias, name: names.get(alias) ?? alias, kind });
    }
    if (wanted.size === 0) throw new Error('cook surfaces: no surface loc types in loc.pack');
    const rows: { x: number; z: number; level: number; name: string; debugname: string; kind: CookKind }[] = [];
    const ids = new Set(wanted.keys());
    for (const map of maps) {
        const { mx, mz } = parseMapsquarePath(map);
        const text = fs.readFileSync(path.join(content, map), 'utf8');
        const linkBelow = parseJm2LinkBelow(text);
        for (const placement of parseJm2LocPlacements(text, ids)) {
            // LINK_BELOW moves a level-1 loc down a plane; the client drops one
            // that would fall below level 0.
            const level = linkBelow.has(`${placement.lx},${placement.lz}`) ? placement.plane - 1 : placement.plane;
            if (level < 0) continue;
            const tile = worldFromMapsquare(mx, mz, placement.lx, placement.lz, level);
            const type = wanted.get(placement.loc_id)!;
            rows.push({ x: tile.x, z: tile.z, level: tile.plane, name: type.name, debugname: type.debugname, kind: type.kind });
        }
    }
    rows.sort((a, b) => a.level - b.level || a.x - b.x || a.z - b.z);
    return { facts: { rows } };
}


/** Selected content supplies the access identity, footprint, and placement; collision resolves the stand at bind time. */
export function extractBankPlacements(content: string, catalog: CatalogBank[]) {
    const locFiles = contentTreeFiles(content, 'scripts', '.loc');
    const npcFiles = contentTreeFiles(content, 'scripts', '.npc');
    const files = [...locFiles, ...npcFiles];
    const maps = placementMapFiles(content);
    const packedLocs = new Map([...parsePack(fs.readFileSync(path.join(content, 'pack/loc.pack'), 'utf8'))].map(([alias, id]) => [id, alias]));
    const packedNpcs = new Map([...parsePack(fs.readFileSync(path.join(content, 'pack/npc.pack'), 'utf8'))].map(([alias, id]) => [id, alias]));
    const configs = new Map<string, Record<string, string>>();
    for (const file of files) {
        const text = fs.readFileSync(path.join(content, file), 'utf8');
        for (const [alias, config] of parseNpcConfigSections(text)) {
            const key = `${path.extname(file)}:${alias}`;
            if (configs.has(key)) throw new Error(`bank placements: duplicate ${key}`);
            configs.set(key, config);
        }
    }
    const rows: { name: string; kind: string; id: number; x: number; z: number; level: number; width: number; length: number }[] = [];
    const ids = new Set(packedLocs.keys());
    const near = (bank: CatalogBank, tile: { x: number; z: number; plane: number }) =>
        bank.tile.level === tile.plane && Math.max(Math.abs(bank.tile.x - tile.x), Math.abs(bank.tile.z - tile.z)) <= 14;
    const matches = (config: Record<string, string> | undefined, access: { name: string; op: string }) =>
        config?.name === access.name && [1, 2, 3, 4, 5].some(index => config[`op${index}`] === access.op);
    for (const map of maps) {
        const { mx, mz } = parseMapsquarePath(map);
        const text = fs.readFileSync(path.join(content, map), 'utf8');
        for (const placement of parseJm2LocPlacements(text, ids)) {
            const tile = worldFromMapsquare(mx, mz, placement.lx, placement.lz, placement.plane);
            const config = configs.get(`.loc:${packedLocs.get(placement.loc_id)}`);
            for (const bank of catalog) {
                if (bank.npcAccess || !near(bank, tile)) continue;
                const access = bank.access ?? { name: 'Bank booth', op: 'Use-quickly' };
                if (!matches(config, access) && !(access.openFirst && matches(config, access.openFirst))) continue;
                let width = Number(config?.width ?? 1), length = Number(config?.length ?? 1);
                if (placement.angle & 1) [width, length] = [length, width];
                if (![width, length].every(value => Number.isInteger(value) && value > 0)) throw new Error(`bank placements: invalid footprint ${bank.name}`);
                rows.push({ name: bank.name, kind: 'object', id: placement.loc_id, x: tile.x, z: tile.z, level: tile.plane, width, length });
            }
        }
        for (const placement of parseJm2NpcPlacements(text)) {
            const tile = worldFromMapsquare(mx, mz, placement.lx, placement.lz, placement.plane);
            const config = configs.get(`.npc:${packedNpcs.get(placement.npc_id)}`);
            for (const bank of catalog) {
                if (bank.npcAccess && near(bank, tile) && matches(config, bank.npcAccess)) {
                    const size = Number(config?.size ?? 1);
                    if (!Number.isInteger(size) || size < 1) throw new Error(`bank placements: invalid NPC size ${bank.name}`);
                    rows.push({ name: bank.name, kind: 'npc', id: placement.npc_id, x: tile.x, z: tile.z, level: tile.plane, width: size, length: size });
                }
            }
        }
    }
    rows.sort((a, b) => catalog.findIndex(bank => bank.name === a.name) - catalog.findIndex(bank => bank.name === b.name) || a.x - b.x || a.z - b.z || a.id - b.id);
    const missing = catalog.filter(bank => !rows.some(row => row.name === bank.name)).map(bank => bank.name);
    return { facts: { rows, missing } };
}

export const DEBUG_SCHEMA_VERSION = 1;
export const DEBUG_STAT_RELATIVE = 'src/engine/entity/PlayerStat.ts';
export type DebugArtifact = {
    schema_version: 1;
    revision: number;
    debug_commands: unknown[];
    debug_names: Record<string, unknown[]>;
};

type BuiltinFishingCandidateMethod = Pick<MethodWire, 'skill' | 'op' | 'targets'>;

export function runtimeNpcOperations(
    evidence: NpcTransformClosureEvidence,
    methods: readonly BuiltinFishingCandidateMethod[],
    hazardNpcs: readonly number[],
) {
    const accepted = evidence.engine.complete ? evidence.proofs.filter(proof =>
        proof.status === 'unchanged' || proof.status === 'consistent') : [];
    return {
        schema: 2,
        operations: accepted.map(proof => {
            const action = evidence.actions.find(row => row.npc_type_id === proof.npc_type_id && row.slot === proof.slot);
            if (!action || action.state !== 'present' || !action.operation_fingerprint || action.trigger_code === null
                || proof.evidence_refs.length === 0
                || proof.unresolved_refs.length || proof.mismatched_npc_type_ids.length
                || !proof.reachable_npc_type_ids.includes(proof.npc_type_id)
                || proof.reachable_npc_type_ids.some(id =>
                    !evidence.npc_types.some(row => row.npc_type_id === id && row.config_state === 'complete')
                    || evidence.actions.every(row =>
                        row.npc_type_id !== id || row.slot !== proof.slot || row.state !== 'present'
                        || row.trigger_code !== action.trigger_code
                        || row.operation_fingerprint !== action.operation_fingerprint))) {
                throw new Error('server_ordered: positive NPC proof disagrees with its action closure');
            }
            return { npc_type_id: action.npc_type_id, slot: action.slot, operation_fingerprint: action.operation_fingerprint };
        }),
        builtin_gatherer_fishing_operations: runtimeBuiltinGathererFishingOperations(evidence, methods, hazardNpcs),
    };
}

export function runtimeBuiltinGathererFishingOperations(
    evidence: NpcTransformClosureEvidence,
    methods: readonly BuiltinFishingCandidateMethod[],
    hazardNpcs: readonly number[],
) {
    if (!evidence.engine.complete) return [];
    const excludedNpcTypes = new Set([390, 403, ...hazardNpcs]);
    const npcTypes = new Map(evidence.npc_types.map(row => [row.npc_type_id, row] as const));
    const actions = new Map(evidence.actions.map(row => [`${row.npc_type_id}:${row.slot}`, row] as const));
    const accepted = new Map<string, { npc_type_id: number; slot: number; operation_fingerprint: string }>();
    for (const method of methods) {
        const op = method.op;
        const targets = method.targets;
        if (method.skill !== 'fishing' || !op || targets.state !== 'known') continue;
        for (const target of targets.value) {
            if (target.kind !== 'npc' || target.class !== 'resource' || target.op !== op.slot
                || excludedNpcTypes.has(target.id)) continue;
            const npcType = npcTypes.get(target.id);
            const action = actions.get(`${target.id}:${target.op}`);
            if (npcType?.config_state !== 'complete' || !action || action.state !== 'present'
                || action.trigger_code === null || !action.operation_fingerprint) continue;
            accepted.set(`${target.id}:${target.op}`, {
                npc_type_id: target.id,
                slot: target.op,
                operation_fingerprint: action.operation_fingerprint,
            });
        }
    }
    return [...accepted.values()].sort((left, right) =>
        left.npc_type_id - right.npc_type_id || left.slot - right.slot);
}

export function buildDebugArtifact(
    revision: number,
    catalog: { commands: unknown[]; names: Record<string, unknown[]> },
): DebugArtifact {
    if (Object.hasOwn(catalog.names, 'obj') || Object.hasOwn(catalog.names, 'namedobj')) {
        throw new Error(`${revision}: debug artifact must not duplicate base item names`);
    }
    return {
        schema_version: DEBUG_SCHEMA_VERSION,
        revision,
        debug_commands: catalog.commands,
        debug_names: catalog.names,
    };
}
export function debugArtifactBytes(artifact: DebugArtifact): string {
    if (artifact.schema_version !== DEBUG_SCHEMA_VERSION) throw new Error(`${artifact.revision}: invalid debug artifact schema`);
    return `${JSON.stringify(artifact, null, 2)}\n`;
}

const hiddenSites = new Map<number, GatherSiteWire[]>();
export type GenerationStage = 1 | 2;

async function generate(spec: Revision, stage: GenerationStage) {
    // Revision 274 predates PvP V0 (PVP-DESIGN-1026): its core omits those facts.
    const pvpV0 = spec.revision !== 274;
    for (const requiredInput of ['data/pack/server/obj.dat', 'data/pack/server/npc.dat', 'data/pack/client/config']) {
        if (!fs.existsSync(path.join(spec.engine, requiredInput))) throw new Error(`${spec.revision}: missing ${requiredInput}`);
    }
    // The selected engine root chooses the decoder module; static imports cannot target each revision.
    process.chdir(spec.engine);
    const environment = (await import(pathToFileURL(path.join(spec.engine, 'src/util/Environment.ts')).href)) as { default: { node?: { members: boolean }; NODE_MEMBERS?: boolean } };
    if ((environment.default.node?.members ?? environment.default.NODE_MEMBERS) !== true) throw new Error(`${spec.revision}: generate with a members world config (NODE_MEMBERS) so tradeable is decoded correctly`);
    const objModule = (await import(pathToFileURL(path.join(spec.engine, 'src/cache/config/ObjType.ts')).href)) as { default: { load(dir: string): void; configs: ObjType[] } };
    objModule.default.load('data/pack');
    const npcModule = (await import(pathToFileURL(path.join(spec.engine, 'src/cache/config/NpcType.ts')).href)) as { default: { load(dir: string): void; configs: NpcType[] } };
    npcModule.default.load('data/pack');

    const categoriesById = new Map<number, string>();
    for (const [name, id] of parsePack(fs.readFileSync(path.join(spec.content, 'pack/category.pack'), 'utf8'))) categoriesById.set(id, name);
    const piles = pileModels(objModule.default.configs);
    const destroyDeathParam = parsePack(fs.readFileSync(path.join(spec.content, 'pack/param.pack'), 'utf8')).get('destroy_death');
    const items = objModule.default.configs.map((obj) => row(obj, piles, categoriesById, destroyDeathParam, pvpV0));
    const aliases = items.filter((item) => item.alias !== null).map((item) => item.alias as string);
    if (new Set(items.map((item) => item.id)).size !== items.length || new Set(aliases).size !== aliases.length) {
        throw new Error(`${spec.revision}: duplicate ids or aliases`);
    }

    const debugHandlerText = fs.readFileSync(path.join(spec.engine, engineHandlerRelative()), 'utf8');
    const debugStatText = fs.readFileSync(path.join(spec.engine, DEBUG_STAT_RELATIVE), 'utf8');
    const debugCatalog = extractDebugCatalog(spec.content, debugHandlerText, debugStatText, npcModule.default.configs);
    const facts = extractFacts(spec.content, objModule.default.configs, npcModule.default.configs);
    const drops = extractDropFacts(spec.content, objModule.default.configs, npcModule.default.configs, spec.revision);
    const magic = extractMagicFacts(spec.content, objModule.default.configs);
    const herbs = extractHerbFacts(spec.content, objModule.default.configs, spec.revision);
    const autocast = spec.revision === 377 ? undefined : extractAutocastControls(spec.content);
    const duel = extractDuelControls(spec.content);
    const special = spec.revision === 377 ? undefined : extractSpecialControls(spec.content, objModule.default.configs);
    const teleports = extractTeleportSpells(spec.content, objModule.default.configs);
    const prayer = extractPrayerFacts(spec.content);
    const nurmofEssence = spec.revision === 377 ? undefined : extractNurmofEssenceFacts(spec.content, objModule.default.configs, npcModule.default.configs);
    const flourSix = spec.revision === 377 ? undefined : extractFlourSixFacts(spec.content, objModule.default.configs);
    const objPackPath = path.join(spec.content, 'pack/obj.pack');
    if (!fs.existsSync(objPackPath)) throw new Error(`${spec.revision}: missing pack/obj.pack`);
    const objPack = parsePack(fs.readFileSync(objPackPath, 'utf8'));
    if (objPack.size === 0) throw new Error(`${spec.revision}: empty pack/obj.pack`);
    const equipmentNames = extractEquipmentNamesFacts(items, objPack, root, spec.revision);
    const gathering = extractGatheringFamily(spec.content, spec.revision);
    const gatheringFile = path.join(path.dirname(spec.output), String(spec.revision), 'gathering.json');
    fs.mkdirSync(path.dirname(gatheringFile), { recursive: true });
    fs.writeFileSync(gatheringFile, familyBytes(gathering, 'gathering'));

    const questIdentity = extractQuestIdentityFacts(spec.content, spec.revision);
    const questStarts = extractQuestStartFacts(spec.content, questIdentity.rows, spec.revision);
    Object.assign(facts, { quest_starts: questStarts });
    if (spec.revision !== 377) Object.assign(facts, { dialogue_ui: extractDialogueUiFacts(spec.content) });
    const npcEvidence = spec.revision === 289 ? extractNpcTransformClosure(spec.engine, spec.content) : undefined;
    const serverOrdered = spec.revision === 289 ? {
        ...await loadServerOrderedFacts({
            contentRoot: spec.content,
            engineRoot: spec.engine,
            revision: spec.revision,
        }),
        npc_transforms: runtimeNpcOperations(npcEvidence!, gathering.payload.methods, gathering.payload.hazard_npcs),
    } : undefined;
    if (serverOrdered) Object.assign(facts, { server_ordered: serverOrdered });
    const combatScripts = parseCombatScripts(spec.content);
    const spellMaxHits = buildSpellMaxHits(spec.content);
    const npcTypeSource = fs.readFileSync(path.join(spec.engine, 'src/cache/config/NpcType.ts'), 'utf8');
    const npcNames = extractNpcNamesFacts(spec.content, spec.revision, npcTypeSource, combatScripts, spellMaxHits);
    const combatStyles = extractCombatStyleFacts(spec.content, objModule.default.configs, npcNames.combatNpcs, combatScripts, pvpV0, spec.revision);
    // PvP V0 facts use content aliases to join decoded obj configurations.
    const aliasToId = new Map(items.filter((item) => item.alias !== null).map((item) => [item.alias as string, item.id]));
    const idToAlias = new Map(items.filter((item) => item.alias !== null).map((item) => [item.id, item.alias as string]));

    Object.assign(facts, {
        style_seqs: combatStyles.style_seqs,
        style_spotanims: combatStyles.style_spotanims,
        combat_tabs: combatStyles.combat_tabs,
        weapon_styles: combatStyles.weapon_styles,
        melee_modes: combatStyles.melee_modes,
        melee_mode_varp: combatStyles.melee_mode_varp,
        ranged_weapons: combatStyles.ranged_weapons,
        ranged_ammo: combatStyles.ranged_ammo,
        ranged_modes: combatStyles.ranged_modes,
        ranged_mode_varp: combatStyles.ranged_mode_varp,
    });

    let meleeWeaponTiers: MeleeWeaponTierFacts | undefined;
    let duelRuleButtons: DuelRuleButtonWire[] | undefined;
    let ruleButtons: number[] | undefined;
    if (pvpV0) {
        const specialWeapons = spec.revision === 377
            ? undefined
            : extractSpecialWeaponFacts(spec.content, objModule.default.configs, combatStyles.weapon_styles, aliasToId);
        const itemCombatBonus = extractItemCombatBonus(spec.content, objModule.default.configs);
        meleeWeaponTiers = extractMeleeWeaponTiers(spec.content, combatStyles.weapon_styles, idToAlias, spec.revision);
        duelRuleButtons = extractDuelRuleButtons(spec.content, spec.revision);
        if (spec.revision !== 377 && !duelRuleButtons.every((button, index) => button.bit === index)) throw new Error(`${spec.revision}: duel rule bits are not contiguous`);
        ruleButtons = Array<number>(14).fill(0);
        for (const button of duelRuleButtons) ruleButtons[button.bit] = button.component;
        Object.assign(duel, { rule_buttons: ruleButtons });
        const duelZones = extractDuelZones(spec.content);
        Object.assign(facts, {
            special_weapons: specialWeapons,
            item_combat_bonus: itemCombatBonus,
            melee_weapon_tiers: meleeWeaponTiers.rows,
            duel_zones: duelZones,
        });
    }

    const locNames = extractLocNamesFacts(spec.content);
    const npcPlacements = extractNpcPlacementsFacts(spec.content);
    const karamja = extractKaramjaFacts(spec.content, npcNames, locNames, npcPlacements, spec.revision);
    Object.assign(facts, { karamja: karamja.facts });
    if (spec.revision === 274 && (questIdentity.coverage.length !== 1 || questIdentity.coverage[0].alias !== 'routequest' || questIdentity.coverage[0].other_pin_id !== 387 || questIdentity.coverage[0].copied !== false)) {
        throw new Error(`${spec.revision}: quest coverage mismatch`);
    }
    if (spec.revision === 289 && questIdentity.coverage.length !== 0) throw new Error(`${spec.revision}: quest coverage must be empty`);
    const quests: FamilyDraft = { schema: QUESTS_SCHEMA, payload: questIdentity };
    const questsFile = path.join(path.dirname(spec.output), String(spec.revision), 'quests.json');
    fs.mkdirSync(path.dirname(questsFile), { recursive: true });
    fs.writeFileSync(questsFile, familyBytes(quests, 'quests'));
    let prereqStepsFile: string | undefined;
    if (stage === 2 && spec.revision === 289) {
        const prereqSteps = extractPrereqStepsFacts(spec.revision);
        assertPrereqStepFacts(spec.content, prereqSteps, questIdentity.rows);
        const prereqFamily: FamilyDraft = { schema: PREREQ_STEPS_SCHEMA, payload: prereqSteps };
        prereqStepsFile = path.join(path.dirname(spec.output), String(spec.revision), 'prereq_steps.json');
        fs.writeFileSync(prereqStepsFile, familyBytes(prereqFamily, 'prereq_steps'));
    }

    const trails = extractTrailFacts(spec.content, objModule.default.configs, spec.revision);
    const talkKey = extractTalkKeyFacts(spec.content, objModule.default.configs);
    assertTalkKeyNpcJoins(talkKey, npcModule.default.configs);
    const trioGivers = extractTrioGiversFacts(spec.content, spec.revision);
    assertTrioGiverNpcJoins(trioGivers, npcModule.default.configs);

    const rs2b0tRoot = requireEnvPath('RS2B0T', 'the rs2b0t source root');
    const bankSource = path.join(rs2b0tRoot, 'src/bot/api/bank/BankLocations.ts');
    const bankCatalog = await extractBankCatalog(spec.engine, fs.readFileSync(bankSource, 'utf8'));
    const bankPlacements = extractBankPlacements(spec.content, bankCatalog);
    if (spec.revision !== 377) fs.writeFileSync(path.join(root, 'crates/api/data/game-data/bank-catalog.json'), bankCatalogJson(bankCatalog));
    const cookFiles = {
        cookLocations: 'src/bot/data/cookLocations.ts',
        cookingRanges: 'src/bot/data/cookingRanges.ts',
        genCookSurfaces: 'tools/cooking/gen-cooksurfaces.ts',
    };
    const cookCatalog = spec.revision === 377 ? undefined : await extractCookCatalog(spec.engine, {
        cookLocations: fs.readFileSync(path.join(rs2b0tRoot, cookFiles.cookLocations), 'utf8'),
        cookingRanges: fs.readFileSync(path.join(rs2b0tRoot, cookFiles.cookingRanges), 'utf8'),
        genCookSurfaces: fs.readFileSync(path.join(rs2b0tRoot, cookFiles.genCookSurfaces), 'utf8'),
    });
    const cookSurfaces = extractCookSurfaces(spec.content, cookCatalog, spec.revision);
    if (cookCatalog) fs.writeFileSync(path.join(root, 'crates/api/data/game-data/cook-catalog.json'), cookCatalogJson(cookCatalog));
    fs.rmSync(path.join(root, 'crates/api/data/game-data/bank-catalog.rs'), { force: true });
    fs.rmSync(path.join(root, 'crates/api/data/game-data/cook-catalog.rs'), { force: true });
    const shops = extractShopFacts(spec.content, items, spec.revision);
    const gatherItemNames = new Map<number, string>();
    for (const item of items) if (item.name !== null && item.name !== '') gatherItemNames.set(item.id, item.name);
    const gatherResourceRows = gatherResources(gathering.payload, gatherItemNames);
    const gatherSiteResult = gatherSites(gathering.payload, gatherResourceRows, spec.content, bankCatalog, { revision: spec.revision });
    if (spec.revision !== 377) hiddenSites.set(spec.revision, gatherSiteResult.hidden);
    Object.assign(facts, { gather_resources: gatherResourceRows, gather_sites: gatherSiteResult.rows });

    const payload = {
        schema_version: 5,
        revision: spec.revision,
        items,
        shops,
        ...facts,
        drop_tables: drops,
        ...magic,
        ...herbs,
        ...prayer,
        nurmof_essence: nurmofEssence,
        flour_six: flourSix,
        equipment_names: equipmentNames,
        quest_identity: questIdentity,
        npc_names: { combat_facts_version: npcNames.combat_facts_version, rows: npcNames.rows },
        loc_names: { rows: locNames.rows },
        npc_placements: { rows: npcPlacements.rows },
        trails,
        talk_key: talkKey,
        trio_givers: trioGivers,
        autocast,
        duel,
        special,
        teleports,
        bank_placements: bankPlacements.facts,
        cook_surfaces: cookSurfaces.facts,
        mining_hazards: miningHazards(gathering.payload),
    };
    if (spec.revision === 377) Object.assign(payload, { coverage: [
        { family: 'autocast', status: 'UNKNOWN', reason: '377 uses autocast_set/autocast_spell; the legacy single-varp armed/selected contract is not derived' },
        { family: 'special', status: 'UNKNOWN', reason: 'legacy arm_confirm_ticks is a host tactic, not a 377 content fact' },
        { family: 'special_weapons', status: 'UNKNOWN', reason: 'SPECIAL_SPECS is a frozen 14-row PvP effect table rather than extraction from selected scripts; 377 also has additional effects such as the Abyssal whip energy drain, so no legacy effect rows are copied' },
        ...(meleeWeaponTiers?.unknown.length ? [{ family: 'melee_weapon_tiers', status: 'UNKNOWN', reason: meleeWeaponTiers.unknown.map(row => `${row.alias} (${row.obj_id}): ${row.reason}`).join('; ') }] : []),
        ...(duelRuleButtons && duelRuleButtons.length < 14 ? [{ family: 'duel.rule_buttons', status: 'UNKNOWN', reason: `selected settings handlers prove ${duelRuleButtons.length} of the 14 declared bits; unavailable button slots ${(ruleButtons ?? []).flatMap((id, bit) => id === 0 ? [bit] : []).join(', ')} stay zero and the API refuses them` }] : []),
        { family: 'nurmof_essence', status: 'UNKNOWN', reason: 'legacy shop/essence family embeds frozen 289 bank and stand tactics' },
        { family: 'flour_six', status: 'UNKNOWN', reason: 'legacy family embeds frozen 289 stand and bank tactics' },
        { family: 'trails.access', status: 'UNKNOWN', reason: '289 curated constrained-clue annotation is not a 377 script proof' },
        { family: 'cook_tactics', status: 'UNKNOWN', reason: 'cooking surfaces derive selected source categories and map placements; frozen 289 stand and camp pairings are not 377 facts' },
        { family: 'gather_sites.access_and_reachability', status: 'UNKNOWN', reason: '377 sites derive selected resource placements and geometry; no 274/289 quest/access curation or nav reachability proof is inherited' },
        { family: 'npc_names.attack_kind', status: 'UNKNOWN', reason: 'the nullable attack_kind schema cannot distinguish source-proven melee from unclassified custom attack branches; unsupported branches are not fabricated as melee and dynamic max-hit gaps remain null' },
        { family: 'spells.runtime_maxhit', status: 'UNKNOWN', reason: 'maxhit records the selected spell DB value, not a full runtime damage proof; Magic Dart uses stat(magic) / 10 + 10 and equipment/target modifiers are not encoded by that scalar' },
        { family: 'equipment_names.ambiguous_queries', status: 'UNKNOWN', reason: 'same-name eligible variants retain candidate identities with disposition ambiguous and no selected id; the frozen display-name query does not establish a preferred 377 variant' },
        { family: 'dialogue_ui', status: 'UNKNOWN', reason: '377 opens questscroll for Seaslug/Watchtower and questscroll_death for the default completion branch; the legacy single common quest_scroll_root contract does not represent both' },
        { family: 'server_ordered', status: 'UNKNOWN', reason: 'existing route and NPC-transform proof is owned by revision 289' },
    ] });
    const debugArtifact = buildDebugArtifact(spec.revision, debugCatalog);
    const debugFile = path.join(path.dirname(spec.output), String(spec.revision), 'debug.json');
    fs.mkdirSync(path.dirname(debugFile), { recursive: true });
    fs.writeFileSync(debugFile, debugArtifactBytes(debugArtifact));
    const bytes = `${JSON.stringify(payload, null, 2)}\n`;
    fs.mkdirSync(path.dirname(spec.output), { recursive: true });
    fs.writeFileSync(spec.output, bytes);
    return {
        revision: spec.revision,
        output: path.relative(root, spec.output),
        records: items.length,
        shop_records: shops.length,
        families: [
            'gathering',
            'quests',
            ...(prereqStepsFile ? ['prereq_steps'] : []),
            'debug',
        ],
        gathering: gathering.summary,
        bank_placements: { rows: bankPlacements.facts.rows.length, missing: bankPlacements.facts.missing.length },
        cook_surfaces: cookSurfaces.facts.rows.length,
    };
}
export type GenerationOptions = { stage: GenerationStage; revisions: number[] };
export function parseGenerationArgs(args: string[]): GenerationOptions {
    let stage: GenerationStage | undefined;
    let revision: number | undefined;
    for (let index = 0; index < args.length; index++) {
        const flag = args[index];
        const value = args[index + 1];
        if (flag === '--stage' && (value === '1' || value === '2') && stage === undefined) {
            stage = Number(value) as GenerationStage;
            index++;
        } else if (flag === '--revision' && ['274', '289', '377'].includes(value ?? '') && revision === undefined) {
            revision = Number(value);
            index++;
        } else {
            throw new Error('usage: --stage 1|2 [--revision 274|289|377]');
        }
    }
    if (stage === undefined) throw new Error('usage: --stage 1|2 [--revision 274|289|377]');
    return { stage, revisions: revision === undefined ? [274, 289] : [revision] };
}
async function main() {
    const options = parseGenerationArgs(process.argv.slice(2));
    const results = [];
    for (const spec of revisions.filter(spec => options.revisions.includes(spec.revision))) {
        results.push(await generate(spec, options.stage));
    }
    if (hiddenSites.size > 0) {
        const hiddenPath = path.join(root, HIDDEN_SITES_FILE);
        const hiddenFile: Record<string, HiddenSitesRecord> = fs.existsSync(hiddenPath) ? JSON.parse(fs.readFileSync(hiddenPath, 'utf8')) : {};
        for (const [revision, hidden] of hiddenSites) hiddenFile[String(revision)] = hiddenSitesRecord(hidden);
        fs.writeFileSync(hiddenPath, hiddenSitesBytes(hiddenFile));
    }
    console.log(JSON.stringify({ stage: options.stage, revisions: results }, null, 2));
}
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) main().catch((error) => { console.error(error); process.exitCode = 1; });
