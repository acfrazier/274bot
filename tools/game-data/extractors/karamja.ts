import {
    parseJm2LocPlacements,
    parseMapsquarePath,
    placementMapInputs,
    requireGatherText,
    sourceFile,
    worldFromMapsquare,
} from './common.ts';

type IdentityRow = { id: number; config: string; display?: string | null; ops?: string[] };
type PlacementRow = { npc_id: number; x: number; z: number; plane: number; mapsquare: string };
type SourceInput = { path: string; bytes: number; sha256: string };

export type KaramjaSpawn = {
    config: string;
    x: number;
    z: number;
    plane: number;
};

export type KaramjaFacts = {
    luthas_spawn: KaramjaSpawn;
    crate_spawn: KaramjaSpawn;
    banana_tree_configs: string[];
    banana_tree_spawns: KaramjaSpawn[];
    crate_capacity: number;
    coin_payout: number;
    dialogue: {
        employment: string;
        paid: string;
        incomplete: string;
    };
};

const SOURCES = {
    loc_pack: 'pack/loc.pack',
    npc_pack: 'pack/npc.pack',
    plantation: 'scripts/areas/area_karamja/configs/plantation.loc',
    banana_tree: 'scripts/areas/area_karamja/scripts/banana_tree.rs2',
    banana_crate: 'scripts/quests/quest_hunt/scripts/banana_crate.rs2',
    luthas: 'scripts/quests/quest_hunt/scripts/luthas.rs2',
} as const;

function onlyConfig(rows: IdentityRow[], config: string, family: string): IdentityRow {
    const matches = rows.filter((row) => row.config === config);
    if (matches.length !== 1) throw new Error(`karamja: expected one ${family} config ${config}, got ${matches.length}`);
    return matches[0];
}

function onlyHandlerConfig(text: string, pattern: RegExp, label: string): string {
    const configs = [...text.matchAll(pattern)].map((match) => match[1]);
    const unique = [...new Set(configs)];
    if (unique.length !== 1) throw new Error(`karamja: expected one ${label} handler config, got ${unique.length}`);
    return unique[0];
}

function locStates(text: string) {
    const states = new Map<string, { category: string | null; next: string | null }>();
    let current: { category: string | null; next: string | null } | null = null;
    for (const raw of text.split(/\r?\n/)) {
        const line = raw.trim();
        if (!line || line.startsWith('//')) continue;
        if (line.startsWith('[') && line.endsWith(']')) {
            const config = line.slice(1, -1);
            if (states.has(config)) throw new Error(`karamja: duplicate loc config [${config}]`);
            current = { category: null, next: null };
            states.set(config, current);
            continue;
        }
        if (!current) continue;
        const equals = line.indexOf('=');
        if (equals < 1) continue;
        const key = line.slice(0, equals);
        const value = line.slice(equals + 1);
        if (key === 'category') current.category = value;
        if (key === 'param') {
            const [name, target, ...extra] = value.split(',');
            if (name !== 'next_loc_stage') continue;
            if (!target || extra.length > 0 || current.next !== null) {
                throw new Error('karamja: malformed or duplicate next_loc_stage param');
            }
            current.next = target;
        }
    }
    return states;
}

function bananaTreeConfigs(plantation: string, treeScript: string): string[] {
    if (!/^\[oploc1,_banana_tree\]$/m.test(treeScript)
        || !/^\[oploc1,bananatreeempty\]$/m.test(treeScript)
        || !/\bloc_change\s*\(\s*loc_param\s*\(\s*next_loc_stage\s*\)\s*,\s*500\s*\)/.test(treeScript)
        || !/\binv_add\s*\(\s*inv\s*,\s*banana\s*,\s*1\s*\)/.test(treeScript)) {
        throw new Error('karamja: banana_tree.rs2 no longer proves the fruiting-state harvest transition');
    }
    const configs = locStates(plantation);
    const treeRows = [...configs].filter(([, row]) => row.category === 'banana_tree');
    if (treeRows.length === 0) throw new Error('karamja: no fruiting banana tree states');
    const treeNames = new Set(treeRows.map(([config]) => config));
    const targets = new Set(treeRows.flatMap(([, row]) =>
        row.next !== null && treeNames.has(row.next) ? [row.next] : []));
    const heads = treeRows.filter(([config]) => !targets.has(config));
    const exits = treeRows.filter(([, row]) => row.next !== null && !treeNames.has(row.next));
    if (heads.length !== 1 || exits.length !== 1) {
        throw new Error(`karamja: banana tree progression needs one head and one empty-state transition, got ${heads.length}/${exits.length}`);
    }
    const terminalConfig = exits[0][1].next!;
    const terminal = configs.get(terminalConfig);
    if (terminalConfig !== 'bananatreeempty'
        || !terminal
        || terminal.category === 'banana_tree'
        || terminal.next !== null) {
        throw new Error('karamja: banana tree progression does not end at the terminal empty state');
    }
    const chain: string[] = [];
    const seen = new Set<string>();
    let config: string | null = heads[0][0];
    while (config !== terminalConfig) {
        if (config === null) throw new Error('karamja: banana tree progression has no terminal state');
        if (seen.has(config)) throw new Error(`karamja: banana tree progression cycles at ${config}`);
        const row = configs.get(config);
        if (!row || row.category !== 'banana_tree') {
            throw new Error(`karamja: banana tree progression targets unknown fruiting state ${config}`);
        }
        seen.add(config);
        chain.push(config);
        config = row.next;
    }
    if (chain.length !== treeRows.length) throw new Error('karamja: banana tree progression has disconnected states');
    return chain;
}

function crateCapacity(crateScript: string, luthasScript: string): number {
    const thresholds = (text: string) => [...text.matchAll(/if\s*\(\s*%crate_bananas\s*=\s*(\d+)\s*\)/g)]
        .map((match) => Number(match[1]))
        .filter((value) => value > 0);
    const crateValues = [...new Set(thresholds(crateScript))];
    if (crateValues.length !== 1) throw new Error(`karamja: expected one positive banana crate capacity, got ${crateValues.join(',') || 'none'}`);
    const capacity = crateValues[0];
    if (!thresholds(luthasScript).includes(capacity)) {
        throw new Error(`karamja: Luthas completion threshold does not match crate capacity ${capacity}`);
    }
    return capacity;
}

function coinPayout(text: string, capacity: number): number {
    const branch = new RegExp(
        `if\\s*\\(\\s*%crate_bananas\\s*=\\s*${capacity}\\s*\\)\\s*\\{([\\s\\S]*?)@multi4\\(`,
    ).exec(text)?.[1];
    if (branch === undefined) throw new Error('karamja: Luthas payout is not under the full-crate condition');
    const messages = [...branch.matchAll(/mes\(\s*"Luthas hands you (\d+) coins\."\s*\)\s*;/g)];
    const grants = [...branch.matchAll(/\binv_add\(\s*inv\s*,\s*coins\s*,\s*(\d+)\s*\)\s*;/g)];
    if (messages.length !== 1 || grants.length !== 1) {
        throw new Error(`karamja: expected one full-crate coin message and grant, got ${messages.length}/${grants.length}`);
    }
    const messageAmount = Number(messages[0][1]);
    const grantAmount = Number(grants[0][1]);
    if (!Number.isSafeInteger(messageAmount) || messageAmount <= 0 || grantAmount !== messageAmount) {
        throw new Error(`karamja: Luthas coin message/grant disagree (${messageAmount}/${grantAmount})`);
    }
    return grantAmount;
}

function quotedOptions(body: string, count: number): string[] {
    const options = [...body.matchAll(/"((?:[^"\\]|\\.)*)"/g)].map((match) => {
        let decoded = '';
        const raw = match[1];
        for (let i = 0; i < raw.length; i += 1) {
            if (raw[i] !== '\\') {
                decoded += raw[i];
                continue;
            }
            i += 1;
            if (i >= raw.length || (raw[i] !== '"' && raw[i] !== '\\')) {
                throw new Error('karamja: unsupported escape in Luthas dialogue choice');
            }
            decoded += raw[i];
        }
        return decoded;
    });
    if (options.length !== count || options.some((option) => option.length === 0)) {
        throw new Error(`karamja: @multi${count} has ${options.length} non-empty dialogue choices`);
    }
    return options;
}

function luthasDialogue(text: string, capacity: number) {
    if (!/if\s*\(\s*testbit\(%hunt_store_employed,\s*\^hunt_not_started\)\s*=\s*\^false\s*\)\s*\{[\s\S]*?@multi2\(/.test(text)) {
        throw new Error('karamja: Luthas no longer gates employment with the selected dialogue');
    }
    const paidBranch = new RegExp(`if\\s*\\(\\s*%crate_bananas\\s*=\\s*${capacity}\\s*\\)\\s*\\{[\\s\\S]*?@multi4\\(`);
    if (!paidBranch.test(text)) throw new Error('karamja: Luthas payment dialogue is not under the full-crate condition');
    const calls = [...text.matchAll(/@multi([1-4])\(([\s\S]*?)\);/g)].map((match) => ({
        count: Number(match[1]),
        options: quotedOptions(match[2], Number(match[1])),
    }));
    if (calls.length !== 3 || calls[0].count !== 2 || calls[1].count !== 4 || calls[2].count !== 4) {
        throw new Error(`karamja: expected Luthas dialogue calls @multi2/@multi4/@multi4, got ${calls.map((call) => call.count).join('/') || 'none'}`);
    }
    return {
        employment: calls[0].options[0],
        paid: calls[1].options[1],
        incomplete: calls[2].options[1],
    };
}

function uniquePlacement<T>(rows: T[], key: (row: T) => string, label: string): T {
    const unique = new Map(rows.map((row) => [key(row), row]));
    if (unique.size !== 1) throw new Error(`karamja: expected one ${label} placement, got ${unique.size}`);
    return unique.values().next().value!;
}

export function extractKaramjaFacts(
    content: string,
    npcNames: { rows: IdentityRow[] },
    locNames: { rows: IdentityRow[] },
    npcPlacements: { rows: PlacementRow[] },
): { facts: KaramjaFacts; inputs: SourceInput[] } {
    const luthasScript = requireGatherText(content, SOURCES.luthas);
    const crateScript = requireGatherText(content, SOURCES.banana_crate);
    const treeScript = requireGatherText(content, SOURCES.banana_tree);
    const plantation = requireGatherText(content, SOURCES.plantation);
    const luthasConfig = onlyHandlerConfig(luthasScript, /^\[opnpc1,([^\]]+)\]$/gm, 'Luthas NPC');
    const crateConfig = onlyHandlerConfig(crateScript, /^\[oploc1,([^\]]+)\]$/gm, 'banana crate loc');
    const treeConfigs = bananaTreeConfigs(plantation, treeScript);
    const capacity = crateCapacity(crateScript, luthasScript);
    const dialogue = luthasDialogue(luthasScript, capacity);
    const payout = coinPayout(luthasScript, capacity);
    const luthasId = onlyConfig(npcNames.rows, luthasConfig, 'NPC').id;
    const crateId = onlyConfig(locNames.rows, crateConfig, 'loc').id;
    const treeIds = new Map(treeConfigs.map((config) => [config, onlyConfig(locNames.rows, config, 'loc').id]));
    const configByLocId = new Map<number, string>([[crateId, crateConfig]]);
    for (const [config, id] of treeIds) {
        if (configByLocId.has(id)) throw new Error(`karamja: duplicate loc id ${id} for ${config}`);
        configByLocId.set(id, config);
    }
    const locIds = new Set(configByLocId.keys());
    const spawns = new Map<string, KaramjaSpawn>();
    const maps = placementMapInputs(content);
    for (const map of maps) {
        const { mx, mz } = parseMapsquarePath(map.path);
        const text = requireGatherText(content, map.path);
        for (const placement of parseJm2LocPlacements(text, locIds)) {
            const config = configByLocId.get(placement.loc_id);
            if (config === undefined) throw new Error(`karamja: no config for loc id ${placement.loc_id}`);
            const world = worldFromMapsquare(mx, mz, placement.lx, placement.lz, placement.plane);
            const spawn = { config, x: world.x, z: world.z, plane: world.plane };
            spawns.set(`${config}:${spawn.x}:${spawn.z}:${spawn.plane}`, spawn);
        }
    }
    const allSpawns = [...spawns.values()];
    const luthasPlacement = uniquePlacement(
        npcPlacements.rows.filter((row) => row.npc_id === luthasId),
        (row) => `${row.x}:${row.z}:${row.plane}`,
        'Luthas NPC',
    );
    const plantationSquare = luthasPlacement.mapsquare;
    const plantationSpawns = allSpawns.filter((spawn) =>
        `m${Math.floor(spawn.x / 64)}_${Math.floor(spawn.z / 64)}` === plantationSquare);
    const crateSpawn = uniquePlacement(
        plantationSpawns.filter((spawn) => spawn.config === crateConfig),
        (spawn) => `${spawn.x}:${spawn.z}:${spawn.plane}`,
        "banana crate on Luthas's plantation mapsquare",
    );
    const treeSpawns = plantationSpawns.filter((spawn) => treeIds.has(spawn.config)).sort((a, b) =>
        a.plane - b.plane || a.z - b.z || a.x - b.x || (a.config < b.config ? -1 : a.config > b.config ? 1 : 0));
    if (treeSpawns.length === 0) throw new Error("karamja: no fruiting banana tree placements on Luthas's plantation mapsquare");
    const inputs = [
        ...maps,
        ...[SOURCES.loc_pack, SOURCES.npc_pack, SOURCES.plantation, SOURCES.banana_tree, SOURCES.banana_crate, SOURCES.luthas]
            .map((relative) => sourceFile(content, relative)),
    ].sort((a, b) => a.path < b.path ? -1 : a.path > b.path ? 1 : 0);
    return {
        facts: {
            luthas_spawn: { config: luthasConfig, x: luthasPlacement.x, z: luthasPlacement.z, plane: luthasPlacement.plane },
            crate_spawn: crateSpawn,
            banana_tree_configs: treeConfigs,
            banana_tree_spawns: treeSpawns,
            crate_capacity: capacity,
            coin_payout: payout,
            dialogue,
        },
        inputs,
    };
}

