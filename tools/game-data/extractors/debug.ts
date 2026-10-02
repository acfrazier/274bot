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
 * Destructive engine commands (operator 2026-10-02): red hover and the send
 * confirmation are reserved for commands that irreversibly remove or downgrade
 * account state the player cannot trivially restore — clearing inventory or
 * bank, resetting a quest or stats, lowering a stat, or deleting items.
 * Teleports, gives, setting a stat up, spawning or adding, and info commands
 * are not destructive. Of the engine vocabulary only `minme` always removes:
 * it lowers every skill. The parametric setters (`setstat`, `advancestat`,
 * `setvar`, `setvarother`) take an operator-typed value that is visible in the
 * editor, so they are not flagged; `give*` adds, `tele*` teleports,
 * `locadd`/`npcadd` spawn, and the rest read state, drive the world, or are
 * production-only moderation hidden from local profiles.
 */
export const ENGINE_DEBUG_COMMANDS: readonly DebugCommand[] = [
    command('reload', 'Reload world scripts.'),
    command('rebuild', 'Rebuild world scripts.'),
    command('speed', 'Set the world tick duration in milliseconds.', [arg('ms', 'int')]),
    command('fly', 'Toggle fly movement.'),
    command('naive', 'Toggle naive movement.'),
    command('random', 'Trigger a random event.'),
    command('setvar', 'Set a player variable.', [arg('var', 'varp'), arg('value', 'int')]),
    command('setvarother', 'Set another player variable.', [arg('username', 'string'), arg('var', 'varp'), arg('value', 'int')], true),
    command('getvar', 'Read a player variable.', [arg('var', 'varp')]),
    command('getvarother', 'Read another player variable.', [arg('username', 'string'), arg('var', 'varp')], true),
    command('give', 'Give an item to the player.', [arg('obj', 'obj'), arg('count', 'int', true)]),
    command('givebank', 'Put an item in the player bank.', [arg('obj', 'obj'), arg('count', 'int', true)]),
    command('giveother', 'Give an item to another player.', [arg('username', 'string'), arg('obj', 'obj'), arg('count', 'int', true)], true),
    command('givecrap', 'Fill the inventory with random items.'),
    command('givemany', 'Give 1000 of an item to the player.', [arg('obj', 'obj')]),
    command('broadcast', 'Broadcast a message to the world.', [arg('message', 'string')], true),
    command('reboot', 'Reboot the world immediately.', [], true),
    command('slowreboot', 'Schedule a world reboot.', [arg('seconds', 'int')], true),
    command('serverdrop', 'Disconnect the player.'),
    command('teleother', 'Teleport another player to you.', [arg('username', 'string')], true),
    command('setstat', 'Set a player skill level.', [arg('stat', 'stat'), arg('level', 'int')]),
    command('advancestat', 'Advance a player skill level.', [arg('stat', 'stat'), arg('level', 'int')]),
    command('minme', 'Set all player skills to their minimum.', [], false, true),
    command('locadd', 'Spawn a location at the player.', [arg('loc', 'loc')]),
    command('npcadd', 'Spawn an NPC at the player.', [arg('npc', 'npc')]),
    command('openmain', 'Open a root interface.', [arg('interface', 'interface')]),
    command('openoverlay', 'Open a root overlay interface.', [arg('interface', 'interface')]),
    command('closeoverlay', 'Close the open overlay.'),
    command('snapshot', 'Write a V8 heap snapshot.'),
    command('getcoord', "Show the player's coordinate."),
    command('tele', 'Teleport to a coordinate.', [arg('coord', 'coord')]),
    command('teleto', 'Teleport to another player.', [arg('username', 'string')], true),
    command('setvis', 'Set player visibility.', [arg('level', 'int')], true),
    command('ban', 'Ban another player.', [arg('username', 'string'), arg('minutes', 'int')], true),
    command('mute', 'Mute another player.', [arg('username', 'string'), arg('minutes', 'int')], true),
    command('kick', 'Kick another player.', [arg('username', 'string')], true),
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

type SourceHeader = { kind: string; name: string; index: number; close: number };
type SourceBlock = { body: string };

/**
 * Destructive content commands (operator 2026-10-02): red hover and the send
 * confirmation are reserved for commands that irreversibly remove or downgrade
 * account state the player cannot trivially restore — clearing inventory or
 * bank, resetting a quest or stats, lowering a stat, or deleting items.
 * Teleports, gives, setting a stat up, spawning or adding, and info commands
 * are not destructive. One rule, no per-command list: the patterns match the
 * script ops that remove or regress, never the ones that grant or advance
 * (`inv_add`, `stat_advance`, `stat_boost`, `stat_heal`, quest-complete queues
 * and progress increments stay quiet). Parametric setters (`%var = $value`)
 * stay quiet too: the typed value is visible in the editor, so the flag is
 * reserved for commands that always remove no matter the arguments.
 */
const DESTRUCTIVE_PATTERNS: readonly RegExp[] = [
    /\binv_clear\s*\(\s*(?:bank|inv|worn|\$[A-Za-z_][A-Za-z0-9_]*)\b/i,
    /\binv_del\s*\(/i,
    /\bstat_(?:sub|drain)\s*\(/i,
    /\b(?:damage_player|poison_player|damage_self)\b/i,
    /\breset_all_quests\b/i,
    /%\w+\s*=\s*0\b/,
    /=\s*\^[A-Za-z0-9_]*not_started\b/i,
    /calc\s*\(\s*%[A-Za-z0-9_]+\s*-/,
];

function hasDestructiveEffect(body: string) {
    const code = withoutSourceCommentsAndStrings(body);
    return DESTRUCTIVE_PATTERNS.some((pattern) => pattern.test(code));
}

function withoutSourceCommentsAndStrings(text: string) {
    let quote: '"' | "'" | null = null;
    let escaped = false;
    let lineComment = false;
    let clean = '';
    for (let index = 0; index < text.length; index += 1) {
        const char = text[index];
        if (lineComment) {
            if (char === '\n') {
                lineComment = false;
                clean += '\n';
            } else {
                clean += ' ';
            }
        } else if (quote !== null) {
            if (escaped) {
                escaped = false;
                clean += char === '\n' ? '\n' : ' ';
            } else if (char === '\\') {
                escaped = true;
                clean += ' ';
            } else if (char === quote) {
                quote = null;
                clean += ' ';
            } else {
                clean += char === '\n' ? '\n' : ' ';
            }
        } else if (char === '/' && text[index + 1] === '/') {
            lineComment = true;
            clean += '  ';
            index += 1;
        } else if (char === '"' || char === "'") {
            quote = char;
            clean += ' ';
        } else {
            clean += char;
        }
    }
    return clean;
}

function sourceFallbackCategory(relative: string, alias: string, body: string) {
    const lower = relative.toLowerCase();
    const lowerAlias = alias.toLowerCase();
    const effectBody = withoutSourceCommentsAndStrings(body);
    if (lowerAlias.startsWith('give') || /\b(?:inv|obj)_(?:add|clear|del|set)\b/i.test(effectBody)) return 'Item';
    if (lower.includes('/quests/') || lower.includes('quest')) return 'Quest';
    if (lower.includes('teles') || lower.includes('teleport')) return 'Teleport';
    if (lower.includes('cheat_bank') || lower.includes('cheat_item') || lower.includes('clearinv') || lower.includes('cheat_magic')) return 'Item';
    if (lower.includes('/engine/') || lower.includes('/debug/')) return 'Client & Engine';
    if (lower.includes('cheat_other') || lower.includes('cheat_reset') || lower.includes('cheat_maxme')) return 'Account';
    return 'Client & Engine';
}

function sourceBlockBody(lines: string[], header: SourceHeader, nextHeader: number) {
    const trailing = lines[header.index].slice(header.close + 1).replace(/\s*\/\/.*$/, '').trim();
    const body = lines.slice(header.index + 1, nextHeader).join('\n');
    return [trailing, body].filter(Boolean).join('\n');
}
const MENU_CALL = /\bp_choice\d+(?:_header)?\s*\(/i;

function followedSourceBody(body: string, blocks: ReadonlyMap<string, SourceBlock>) {
    const followed = new Set<string>();
    const collect = (fragment: string, root: boolean): string => {
        const clean = withoutSourceCommentsAndStrings(fragment);
        const menu = root ? null : MENU_CALL.exec(clean);
        const visible = menu ? clean.slice(0, menu.index + menu[0].length) : clean;
        const parts = [visible];
        for (const match of visible.matchAll(/[@~]([a-z][a-z0-9_]*)\b/gi)) {
            const name = match[1].toLowerCase();
            const target = blocks.get(name);
            if (!target || followed.has(name)) continue;
            followed.add(name);
            parts.push(collect(target.body, false));
        }
        return parts.join('\n');
    };
    return collect(body, true);
}


const HEADER_PATTERN = /^\s*\[([a-z][a-z0-9_]*)\s*,\s*([^\],\s]+)\]/i;

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
    const headers: SourceHeader[] = [];
    for (let index = 0; index < lines.length; index += 1) {
        const match = HEADER_PATTERN.exec(lines[index]);
        if (!match) continue;
        headers.push({ kind: match[1].toLowerCase(), name: match[2], index, close: match[0].lastIndexOf(']') });
    }
    const nextHeaderByIndex = new Map<number, number>();
    const headerByIndex = new Map<number, SourceHeader>();
    const blocks = new Map<string, SourceBlock>();
    for (let position = 0; position < headers.length; position += 1) {
        const header = headers[position];
        const nextHeader = headers[position + 1]?.index ?? lines.length;
        nextHeaderByIndex.set(header.index, nextHeader);
        headerByIndex.set(header.index, header);
        if (header.kind === 'label' || header.kind === 'proc') {
            blocks.set(header.name.toLowerCase(), { body: sourceBlockBody(lines, header, nextHeader) });
        }
    }
    const rows: ParsedDebugproc[] = [];
    for (let index = 0; index < lines.length; index += 1) {
        const raw = lines[index];
        const match = /^\s*\[debugproc,\s*([^\],\s]+)\](?:\(([^)]*)\))?\s*(.*)$/.exec(raw);
        if (!match) continue;
        const alias = match[1];
        const wire = `~${alias}`;
        const args = match[2]?.trim() ? match[2].split(',').map((value) => parseArgument(value, `${relative}:${index + 1}`)) : [];
        const header = headerByIndex.get(index);
        if (!header) throw new Error(`${relative}:${index + 1}: malformed debugproc header`);
        const body = sourceBlockBody(lines, header, nextHeaderByIndex.get(index) ?? lines.length);
        const effectBody = followedSourceBody(body, blocks);
        const trailing = match[3].trim();
        const hint = help.get(alias.toLowerCase());
        const inline = /\/\/\s*(.*)$/.exec(trailing)?.[1]?.trim();
        const description = hint?.description ?? inline ?? '';
        rows.push({
            name: wire,
            category: hint?.category ?? sourceFallbackCategory(relative, alias, effectBody),
            description,
            args,
            destructive: hasDestructiveEffect(effectBody),
            production_only: false,
            source: relative,
            line: index + 1,
        });
    }
    return rows;
}

type EngineProductionGate = 'none' | 'production' | 'non-production';
type ParsedEngineCommand = { name: string; productionGate: EngineProductionGate };

const ENGINE_NON_PRODUCTION_ONLY: Record<string, true> = { givebank: true };

function parseEngineCommands(text: string) {
    const found = new Map<string, ParsedEngineCommand>();
    for (const match of text.matchAll(/\bcmd\s*===\s*(['"])([a-z0-9_]+)\1/g)) {
        const index = match.index ?? 0;
        const lineEnd = text.indexOf('\n', index);
        const condition = text.slice(index, lineEnd < 0 ? text.length : lineEnd);
        const productionGate: EngineProductionGate = /&&\s*!\s*Environment\.node\.production\b/.test(condition)
            ? 'non-production'
            : /&&\s*Environment\.node\.production\b/.test(condition)
              ? 'production'
              : 'none';
        found.set(match[2], { name: match[2], productionGate });
    }
    return found;
}

function expectedEngineProductionGate(row: DebugCommand): EngineProductionGate {
    if (row.production_only) return 'production';
    if (ENGINE_NON_PRODUCTION_ONLY[row.name]) return 'non-production';
    return 'none';
}

/** Verify the hand-maintained engine metadata still covers the selected handler. */
export function assertEngineCommandDrift(text: string) {
    const found = parseEngineCommands(text);
    const missing = [...found.keys()].filter((name) => !ENGINE_COMMAND_NAMES.includes(name));
    const stale = [...ENGINE_COMMAND_NAMES].filter((name) => !found.has(name) && !ENGINE_OPTIONAL_COMMAND_NAMES.some((optional) => optional === name));
    const gateDrift = ENGINE_DEBUG_COMMANDS.filter((row) => {
        const source = found.get(row.name);
        return source && source.productionGate !== expectedEngineProductionGate(row);
    }).map((row) => `${row.name}: expected ${expectedEngineProductionGate(row)}`);
    if (missing.length || stale.length || gateDrift.length) {
        const suffix = gateDrift.length ? `, production gate drift [${gateDrift.join(', ')}]` : '';
        throw new Error(`ClientCheatHandler command drift: missing metadata [${missing.join(', ')}], stale metadata [${stale.join(', ')}]${suffix}`);
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
