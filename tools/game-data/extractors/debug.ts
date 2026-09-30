import fs from 'node:fs';
import path from 'node:path';

import { parsePack, sourceFile } from './common.ts';

export type InputHash = { path: string; bytes: number; sha256: string };

export type DebugArgument = {
    name: string;
    kind: string;
    optional: boolean;
};

export type DebugCommand = {
    name: string;
    category: string;
    description: string;
    args: DebugArgument[];
    destructive: boolean;
    production_only: boolean;
};

export type DebugName = {
    id: number;
    alias: string;
    name: string;
};

export type DebugNameFamilies = Record<string, DebugName[]>;

export type DebugCatalog = {
    commands: DebugCommand[];
    names: DebugNameFamilies;
    inputs: InputHash[];
};

const DEBUG_KINDS = new Set([
    'int',
    'string',
    'obj',
    'npc',
    'loc',
    'seq',
    'spotanim',
    'interface',
    'stat',
    'varp',
    'inv',
    'idkit',
    'namedobj',
    'coord',
]);

const CATEGORIES = new Set(['Account', 'Item', 'Teleport', 'Quest', 'Client & Engine']);

const command = (
    name: string,
    description: string,
    args: DebugArgument[] = [],
    production_only = false,
    destructive = false,
): DebugCommand => ({
    name,
    category: 'Client & Engine',
    description,
    args,
    destructive,
    production_only,
});

const arg = (name: string, kind: string, optional = false): DebugArgument => ({ name, kind, optional });

/**
 * The engine owns this small command vocabulary rather than content. Keep its
 * argument metadata beside the table, then compare the names with the pinned
 * handler on every generation. Debugproc names never appear here.
 */
export const ENGINE_DEBUG_COMMANDS: readonly DebugCommand[] = [
    command('reload', 'Reload world scripts.', [], false, true),
    command('rebuild', 'Rebuild world scripts.', [], false, true),
    command('speed', 'Set the world tick duration in milliseconds.', [arg('ms', 'int')]),
    command('fly', 'Toggle fly movement.', [], false, false),
    command('naive', 'Toggle naive movement.', [], false, false),
    command('random', 'Trigger a random event.', [], false, false),
    command('setvar', 'Set a player variable.', [arg('var', 'varp'), arg('value', 'int')], false, true),
    command('setvarother', 'Set another player variable.', [arg('username', 'string'), arg('var', 'varp'), arg('value', 'int')], true, true),
    command('getvar', 'Read a player variable.', [arg('var', 'varp')]),
    command('getvarother', 'Read another player variable.', [arg('username', 'string'), arg('var', 'varp')], true),
    command('give', 'Give an item to the player.', [arg('obj', 'obj'), arg('count', 'int', true)], false, true),
    command('givebank', 'Put an item in the player bank.', [arg('obj', 'obj'), arg('count', 'int', true)], false, true),
    command('giveother', 'Give an item to another player.', [arg('username', 'string'), arg('obj', 'obj'), arg('count', 'int', true)], true, true),
    command('givecrap', 'Fill the inventory with random items.', [], false, true),
    command('givemany', 'Give 1000 of an item to the player.', [arg('obj', 'obj')], false, true),
    command('broadcast', 'Broadcast a message to the world.', [arg('message', 'string')], true, true),
    command('reboot', 'Reboot the world immediately.', [], true, true),
    command('slowreboot', 'Schedule a world reboot.', [arg('seconds', 'int')], true, true),
    command('serverdrop', 'Disconnect the player.', [], false, true),
    command('teleother', 'Teleport another player to you.', [arg('username', 'string')], true, true),
    command('setstat', 'Set a player skill level.', [arg('stat', 'stat'), arg('level', 'int')], false, true),
    command('advancestat', 'Advance a player skill level.', [arg('stat', 'stat'), arg('level', 'int')], false, true),
    command('minme', 'Set all player skills to their minimum.', [], false, true),
    command('locadd', 'Spawn a location at the player.', [arg('loc', 'loc')], false, true),
    command('npcadd', 'Spawn an NPC at the player.', [arg('npc', 'npc')], false, true),
    command('openmain', 'Open a root interface.', [arg('interface', 'interface')], false, true),
    command('openoverlay', 'Open a root overlay interface.', [arg('interface', 'interface')], false, true),
    command('closeoverlay', 'Close the open overlay.', [], false, true),
    command('snapshot', 'Write a V8 heap snapshot.', [], false, true),
    command('getcoord', "Show the player's coordinate.", []),
    command('tele', 'Teleport to a coordinate.', [arg('coord', 'coord')], false, true),
    command('teleto', 'Teleport to another player.', [arg('username', 'string')], true, true),
    command('setvis', 'Set player visibility.', [arg('level', 'int')], true, true),
    command('ban', 'Ban another player.', [arg('username', 'string'), arg('minutes', 'int')], true, true),
    command('mute', 'Mute another player.', [arg('username', 'string'), arg('minutes', 'int')], true, true),
    command('kick', 'Kick another player.', [arg('username', 'string')], true, true),
];

const ENGINE_COMMAND_NAMES = ENGINE_DEBUG_COMMANDS.map((row) => row.name);
const ENGINE_OPTIONAL_COMMAND_NAMES = ['givebank'] as const;
// The 274 handler predates the local-only bank seed command; 289 adds it.

function normalizedCategory(value: string) {
    const trimmed = value.trim();
    if (trimmed === 'Engine commands' || trimmed === 'Client commands' || trimmed === 'Client & Engine commands') {
        return 'Client & Engine';
    }
    const category = trimmed.replace(/\s+commands$/, '');
    return CATEGORIES.has(category) ? category : null;
}

function humanName(alias: string) {
    return alias
        .replaceAll('_', ' ')
        .replaceAll(':', ' ')
        .replace(/\s+/g, ' ')
        .trim()
        .split(' ')
        .map((word) => word ? `${word[0].toUpperCase()}${word.slice(1)}` : word)
        .join(' ');
}

function stripColorTags(text: string) {
    return text.replace(/@[a-z0-9]+@/gi, '').trim();
}

export type HelpEntry = { category: string; description?: string };

/** Extract the category/description hints from the selected content's help proc. */
export function parseDebugHelp(text: string): Map<string, HelpEntry> {
    const entries = new Map<string, HelpEntry>();
    for (const raw of text.split(/\r?\n/)) {
        const start = raw.indexOf('~mesbox("');
        if (start < 0) continue;
        const end = raw.indexOf('");', start + '~mesbox("'.length);
        if (end < 0) continue;
        const message = stripColorTags(raw.slice(start + '~mesbox("'.length, end));
        const sections = message.split('|');
        const category = normalizedCategory(sections[0] ?? '');
        if (!category) continue;
        for (const section of sections.slice(1)) {
            const matches = [...section.matchAll(/::([a-z0-9_]+)(?:\s*-\s*([^/]+?))?(?=(?:\/::|$))/gi)];
            for (const match of matches) {
                const name = match[1].toLowerCase();
                const description = match[2]?.trim();
                entries.set(name, { category, ...(description ? { description } : {}) });
            }
        }
    }
    return entries;
}

function sourceFallbackCategory(relative: string) {
    const lower = relative.toLowerCase();
    if (lower.includes('/quests/') || lower.includes('quest')) return 'Quest';
    if (lower.includes('teles') || lower.includes('teleport')) return 'Teleport';
    if (lower.includes('cheat_bank') || lower.includes('cheat_item') || lower.includes('clearinv') || lower.includes('cheat_magic')) return 'Item';
    if (lower.includes('/engine/') || lower.includes('/debug/')) return 'Client & Engine';
    if (lower.includes('cheat_other') || lower.includes('cheat_reset') || lower.includes('cheat_maxme')) return 'Account';
    return 'Client & Engine';
}

function parseArgument(raw: string, source: string): DebugArgument {
    let value = raw.trim();
    let optional = false;
    if (value.startsWith('[') && value.endsWith(']')) {
        optional = true;
        value = value.slice(1, -1).trim();
    }
    const match = /^(\w+)\s+\$?([a-zA-Z_][a-zA-Z0-9_]*)$/.exec(value);
    if (!match) throw new Error(`${source}: malformed debugproc argument ${raw}`);
    const kind = match[1].toLowerCase();
    if (!DEBUG_KINDS.has(kind)) throw new Error(`${source}: unsupported debugproc argument kind ${kind}`);
    return { name: match[2], kind, optional };
}

export type ParsedDebugproc = DebugCommand & { source: string; line: number };

/** Parse all debugproc headers from one selected `.rs2` source. */
export function parseDebugprocSource(text: string, relative: string, help = new Map<string, HelpEntry>): ParsedDebugproc[] {
    const lines = text.split(/\r?\n/);
    const rows: ParsedDebugproc[] = [];
    for (let index = 0; index < lines.length; index += 1) {
        const raw = lines[index];
        const match = /^\s*\[debugproc,([^\],\s]+)\](?:\(([^)]*)\))?\s*(.*)$/.exec(raw);
        if (!match) continue;
        const alias = match[1];
        const wire = `~${alias}`;
        const args = match[2]?.trim() ? match[2].split(',').map((value) => parseArgument(value, `${relative}:${index + 1}`)) : [];
        const nextHeader = lines.findIndex((line, candidate) => candidate > index && /^\s*\[debugproc,/.test(line));
        const trailing = match[3].trim();
        const bodyStart = trailing.replace(/\s*\/\/.*$/, '').trim();
        const body = [bodyStart, lines.slice(index + 1, nextHeader < 0 ? lines.length : nextHeader).join('\n')].filter(Boolean).join('\n');
        const hint = help.get(alias.toLowerCase());
        const inline = /\/\/\s*(.*)$/.exec(trailing)?.[1]?.trim();
        const description = hint?.description ?? inline ?? '';
        const destructive = /(?:inv_(?:add|clear|del)|obj_add|loc_(?:add|del|change)|npc_(?:add|del)|p_(?:teleport|telejump)|stat_(?:add|sub|boost|drain)|settimer|queue\s*\(|healenergy|send_quest_progress|clear_pk_skull|damage_self|(?:^|[_-])(give|drop|reset|complete|kill|damage|poison|maxme)(?:[_-]|$))/i.test(`${alias}\n${body}`);
        rows.push({
            name: wire,
            category: hint?.category ?? sourceFallbackCategory(relative),
            description,
            args,
            destructive,
            production_only: false,
            source: relative,
            line: index + 1,
        });
    }
    return rows;
}

function parseEngineCommandNames(text: string) {
    const found = new Set<string>();
    for (const match of text.matchAll(/\bcmd\s*===\s*(['"])([a-z0-9_]+)\1/g)) found.add(match[2]);
    return found;
}

/** Verify the hand-maintained engine metadata still covers the selected handler. */
export function assertEngineCommandDrift(text: string) {
    const found = parseEngineCommandNames(text);
    const missing = [...found].filter((name) => !ENGINE_COMMAND_NAMES.includes(name));
    const stale = [...ENGINE_COMMAND_NAMES].filter((name) => !found.has(name) && !ENGINE_OPTIONAL_COMMAND_NAMES.some((optional) => optional === name));
    if (missing.length || stale.length) {
        throw new Error(`ClientCheatHandler command drift: missing metadata [${missing.join(', ')}], stale metadata [${stale.join(', ')}]`);
    }
    return ENGINE_DEBUG_COMMANDS.filter((row) => found.has(row.name)).map((row) => ({ ...row, args: row.args.map((value) => ({ ...value })) }));
}

function walkFiles(directory: string, suffix: string) {
    const files: string[] = [];
    const walk = (absolute: string) => {
        for (const entry of fs.readdirSync(absolute, { withFileTypes: true })) {
            const child = path.join(absolute, entry.name);
            if (entry.isDirectory()) walk(child);
            else if (entry.name.endsWith(suffix)) files.push(child);
        }
    };
    if (fs.existsSync(directory)) walk(directory);
    return files.sort();
}

function sourcePacks(content: string, names: DebugNameFamilies, inputs: InputHash[]) {
    const addPack = (kind: string, relative: string, fallbackNames?: Map<string, DebugName>) => {
        const file = path.join(content, relative);
        if (!fs.existsSync(file)) throw new Error(`debug names: required ${relative} is missing`);
        const pack = parsePack(fs.readFileSync(file, 'utf8'));
        const rows = [...pack.entries()].map(([alias, id]) => fallbackNames?.get(alias) ?? ({ alias, id, name: humanName(alias) }));
        rows.sort((a, b) => a.name.localeCompare(b.name) || a.id - b.id || a.alias.localeCompare(b.alias));
        names[kind] = rows;
        inputs.push(sourceFile(content, relative));
    };
    const addInput = (relative: string) => { const file = path.join(content, relative); if (!fs.existsSync(file)) throw new Error(`debug names: required ${relative} is missing`); inputs.push(sourceFile(content, relative)); };
    addInput('pack/obj.pack');
    const npcFallback = new Map((names.npc ?? []).map((row) => [row.alias, row])); addPack('npc', 'pack/npc.pack', npcFallback);
    addPack('loc', 'pack/loc.pack');
    addPack('seq', 'pack/seq.pack');
    addPack('spotanim', 'pack/spotanim.pack');
    addPack('interface', 'pack/interface.pack');
    addPack('varp', 'pack/varp.pack');
    const varbit = path.join(content, 'pack/varbit.pack');
    if (!fs.existsSync(varbit)) throw new Error('debug names: required pack/varbit.pack is missing');
    names.varp.push(...[...parsePack(fs.readFileSync(varbit, 'utf8')).entries()].map(([alias, id]) => ({ alias, id, name: humanName(alias) })));
    names.varp.sort((a, b) => a.name.localeCompare(b.name) || a.id - b.id || a.alias.localeCompare(b.alias));
    inputs.push(sourceFile(content, 'pack/varbit.pack'));
    addPack('inv', 'pack/inv.pack');
    addPack('idkit', 'pack/idk.pack');
}

function statNames(statText: string) {
    const enumMatch = /enum\s+PlayerStat\s*\{([\s\S]*?)\}/.exec(statText);
    const ids = new Map<string, number>();
    if (enumMatch) {
        let id = 0;
        for (const raw of enumMatch[1].split(',')) {
            const alias = raw.trim().split(/\s+/)[0];
            if (!alias) continue;
            ids.set(alias, id);
            id += 1;
        }
    }
    const names: DebugName[] = [];
    for (const match of statText.matchAll(/\[['"]([A-Z][A-Z0-9_]*)['"]\s*,\s*PlayerStat\.([A-Z0-9_]+)\]/g)) {
        const alias = match[1].toLowerCase();
        if (alias === 'stat18' || alias === 'stat19') continue;
        names.push({ alias, id: ids.get(match[2]) ?? names.length, name: humanName(alias) });
    }
    const deduped = new Map(names.map((row) => [row.alias, row]));
    return [...deduped.values()].sort((a, b) => a.id - b.id || a.alias.localeCompare(b.alias));
}

function addDecodedNames(names: DebugNameFamilies, kind: string, rows: readonly { id: number; debugname?: string | null; name: string | null }[]) {
    names[kind] = rows
        .filter((row) => row.debugname && row.name)
        .map((row) => ({ id: row.id, alias: row.debugname as string, name: row.name as string }))
        .sort((a, b) => a.name.localeCompare(b.name) || a.id - b.id || a.alias.localeCompare(b.alias));
}

function uniqueByAlias(rows: DebugName[]) {
    return [...new Map(rows.map((row) => [row.alias, row])).values()];
}

export function extractDebugCatalog(
    content: string,
    engineHandlerText: string,
    statText: string,
    npcs: readonly { id: number; debugname?: string | null; name: string | null }[],
): DebugCatalog {
    const helpFile = path.join(content, 'scripts/_test/scripts/cheats/cheat_help.rs2');
    const help = fs.existsSync(helpFile) ? parseDebugHelp(fs.readFileSync(helpFile, 'utf8')) : new Map<string, HelpEntry>();
    const debugprocFiles = walkFiles(path.join(content, 'scripts'), '.rs2');
    const commands: ParsedDebugproc[] = [];
    const inputs: InputHash[] = [];
    for (const file of debugprocFiles) {
        const text = fs.readFileSync(file, 'utf8');
        if (!text.includes('[debugproc,')) continue;
        const relative = path.relative(content, file).split(path.sep).join('/');
        const rows = parseDebugprocSource(text, relative, help);
        if (!rows.length) continue;
        commands.push(...rows);
        inputs.push(sourceFile(content, relative));
    }
    const seen = new Set<string>();
    for (const row of commands) {
        if (seen.has(row.name)) throw new Error(`debugproc ${row.name} is declared more than once`);
        seen.add(row.name);
        if (!CATEGORIES.has(row.category)) throw new Error(`debugproc ${row.name}: unknown category ${row.category}`);
    }
    const engine = assertEngineCommandDrift(engineHandlerText).map((row) => { const hint = help.get(row.name); return hint ? { ...row, category: hint.category, description: hint.description ?? row.description } : row; });
    commands.push(...engine);
    commands.sort((a, b) => a.category.localeCompare(b.category) || a.name.localeCompare(b.name));
    const names: DebugNameFamilies = {};
    addDecodedNames(names, 'npc', npcs);
    sourcePacks(content, names, inputs);
    names.stat = statNames(statText);
    names.varp = uniqueByAlias(names.varp ?? []);
    for (const [kind, rows] of Object.entries(names)) {
        rows.sort((a, b) => a.name.localeCompare(b.name) || a.id - b.id || a.alias.localeCompare(b.alias));
        names[kind] = rows;
    }
    const catalogCommands = commands.map(({ source: _source, line: _line, ...row }) => row);
    return { commands: catalogCommands, names, inputs };
}

export function engineHandlerRelative() {
    return 'src/network/game/client/handler/ClientCheatHandler.ts';
}
