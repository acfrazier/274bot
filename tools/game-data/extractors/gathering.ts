// M-306 / M-215 gathering family: typed resource methods, rock classes, respawn facts, tools, bait, products and
// map placements, all joined from the pinned content tree. Unknown constructs become explicit gaps, never guesses.
import fs from 'node:fs';
import path from 'node:path';
import { indexContent, parseDbRows, parseCoord, scanMapSection, spanRef, stripComment, type ContentIndex, type DbRow, type Rs2Block, type Rs2Header, type Section, type Span } from './gathering-content.ts';
import { parseBody, walkStatements, returns, type Node, type PathCond, type Stmt } from './gathering-rs2.ts';

/** Extractor schema. Bump on any change to the wire shape below; the Rust decoder pins the same number. */
export const GATHERING_SCHEMA = 2;

/** Engine `PlayerStat` order (`src/engine/entity/PlayerStat.ts`): a protocol constant, not gathering data. */
const SKILL = { attack: 0, woodcutting: 8, fishing: 10, mining: 14 } as const;
const SKILL_NAMES = { woodcutting: 'woodcutting', mining: 'mining', fishing: 'fishing' } as const;
type SkillName = keyof typeof SKILL_NAMES;

// ---- wire ------------------------------------------------------------------------------------------------

export type Ref = string;
export type Gap = { code: string; sources: Ref[] };
export type Know<T> = { state: 'known'; value: T } | { state: 'partial'; value: T; gaps: Gap[] } | { state: 'unknown'; gap: Gap };
export type TargetClass = 'resource' | 'depleted' | 'hazard' | 'unclassified';
export type ScaleWire = { rule: string; min_ticks: number; max_ticks: number; sources: Ref[] };
export type RespawnWire = { raw: number; scale: Know<ScaleWire>; source: Ref };
export type TargetWire = { kind: 'loc' | 'npc'; id: number; op: number; class: TargetClass; respawn: Know<RespawnWire | null> };
export type YieldWire = { item: number; level: number };
export type GateWire = { skill: number; level: number };
export type ToolWire = { item: number; use_gate: GateWire | null; wield_gate: GateWire | null };
export type AmountWire = { item: number; count: number };
export type RequirementWire = { id: string; source: Ref } & ({ kind: 'skill'; skill: number; level: number } | { kind: 'members' });
export type MethodWire = {
    id: string;
    skill: SkillName;
    resources: string[];
    op: { slot: number; label: string } | null;
    targets: Know<TargetWire[]>;
    products: Know<YieldWire[]>;
    tools: Know<ToolWire[]>;
    consumes: Know<AmountWire[]>;
    requirements: Know<RequirementWire[]>;
    /** `known` means the placement rows of this method's resource targets are complete. */
    spots: Know<null>;
    sources: Ref[];
};
export type RegionWire = { min_x: number; min_z: number; max_x: number; max_z: number; level: number };
export type MovementWire = { npc: number; region: Know<RegionWire | null> };
export type ZoneEffect = 'yield-intercepted' | 'product-substituted' | 'state-gated';
export type ZoneWire = { methods: string[]; level: number; min_x: number; min_z: number; max_x: number; max_z: number; effect: ZoneEffect; sources: Ref[] };
export type LooseWire = { skill: SkillName; kind: 'loc' | 'npc'; id: number; class: 'depleted' | 'unclassified'; gap: Gap | null };
export type PlacementFile = { file: string; rows: string[] };
export type GatheringFacts = {
    entities: string[];
    methods: MethodWire[];
    loose: LooseWire[];
    zones: ZoneWire[];
    movements: MovementWire[];
    placements: PlacementFile[];
    hazard_npcs: number[];
    incidental_gem_ids: number[];
};
export type GatheringSummary = {
    methods: Record<SkillName, number>;
    rocks: { population: number; resource: number; depleted: number; hazard: number; unclassified: number; unclassified_reasons: Record<string, number> };
    targets: number;
    placements: { rows: number; files: number; by_method: Record<string, number> };
    zones: number;
    knowledge: { partial: number; unknown: number };
};

const known = <T>(value: T): Know<T> => ({ state: 'known', value });
const partial = <T>(value: T, gaps: Gap[]): Know<T> => (gaps.length === 0 ? known(value) : { state: 'partial', value, gaps: dedupeGaps(gaps) });
const unknown = <T>(code: string, sources: Ref[]): Know<T> => ({ state: 'unknown', gap: { code, sources } });
const gap = (code: string, ...spans: Span[]): Gap => ({ code, sources: spans.map(spanRef) });
const line = (file: string, number: number): Span => ({ file, first: number, last: number });

function dedupeGaps(gaps: Gap[]) {
    const seen = new Set<string>();
    return gaps.filter((each) => {
        const key = `${each.code}\0${each.sources.join('\0')}`;
        if (seen.has(key)) return false;
        seen.add(key);
        return true;
    });
}

// ---- entities --------------------------------------------------------------------------------------------

type EntityKind = 'loc' | 'npc' | 'obj';
class Entities {
    private readonly rows = new Map<string, { kind: EntityKind; id: number; alias: string; width: number; length: number }>();
    constructor(private readonly idx: ContentIndex) {}
    /** Join an alias to its pack id; the row is published with its footprint (locs only). */
    id(kind: EntityKind, alias: string, label: string) {
        const id = this.idx.packs[kind].get(alias);
        if (id === undefined) throw new Error(`${label}: failed join, ${kind}.pack lacks ${alias}`);
        const key = `${kind}:${id}`;
        if (!this.rows.has(key)) {
            const dimension = (name: string) => {
                const raw = kind === 'loc' ? this.idx.loc.get(alias)?.props.get(name)?.[0] : undefined;
                const value = raw === undefined ? 1 : Number(raw);
                if (!Number.isInteger(value) || value < 1 || value > 255) throw new Error(`${label}: ${alias} ${name} must be 1..255, got ${raw}`);
                return value;
            };
            this.rows.set(key, { kind, id, alias, width: dimension('width'), length: dimension('length') });
        }
        return id;
    }
    alias(kind: EntityKind, id: number) {
        const row = this.rows.get(`${kind}:${id}`);
        if (!row) throw new Error(`${kind} ${id}: entity was never joined`);
        return row.alias;
    }
    lines() {
        const order = { loc: 0, npc: 1, obj: 2 } as const;
        return [...this.rows.values()]
            .sort((a, b) => order[a.kind] - order[b.kind] || a.id - b.id)
            .map((row) => (row.kind === 'loc' ? `loc ${row.id} ${row.alias} ${row.width} ${row.length}` : `${row.kind} ${row.id} ${row.alias}`));
    }
}

type Ctx = { idx: ContentIndex; entities: Entities };

// ---- statement helpers ---------------------------------------------------------------------------------

const RANDOM_EVENT = /^afk_event\s*=\s*\^true\b/;
const isRandomEvent = (cond: string) => RANDOM_EVENT.test(cond);

type Flat = { stmt: Stmt; path: PathCond[] };

/** Every statement below the block's top level, with its path; random-event subtrees are not entered. */
function flatten(block: Rs2Block): { flat: Flat[]; nodes: Node[] } {
    const nodes = parseBody(block.body);
    const flat: Flat[] = [];
    walkStatements(nodes, (stmt, path) => flat.push({ stmt, path }), isRandomEvent);
    return { flat, nodes };
}

function uniqueBlock(ctx: Ctx, kind: string, name: string): Rs2Block | null {
    const found = ctx.idx.headers.get(`${kind},${name}`) ?? [];
    if (found.length !== 1) return null;
    return ctx.idx.blocks(found[0].span.file).find((block) => block.kind === kind && block.name === name && block.span.first === found[0].span.first) ?? null;
}

const blockSpan = (block: Rs2Block) => block.span;
/** Comment-free code text of a block body, lines joined by a space. */
const codeText = (block: Rs2Block) => block.body.map((each) => stripComment(each.text).trim()).filter(Boolean).join(' ');

/** True when `flat` contains a statement whose text matches. */
const hasStmt = (flat: Flat[], pattern: RegExp | string) => flat.some(({ stmt }) => (typeof pattern === 'string' ? stmt.text === pattern : pattern.test(stmt.text)));

/** Every `if` condition that appears anywhere (also in a nested else-if chain). */
function conditions(nodes: Node[], into: string[] = []) {
    for (const node of nodes) {
        if (node.kind === 'if') {
            into.push(node.cond);
            conditions(node.then, into);
            conditions(node.els, into);
        } else if (node.kind === 'while') conditions(node.body, into);
    }
    return into;
}

// ---- respawn scale (scale_by_playercount) --------------------------------------------------------------

type PlayerScale = { cap: number; denominator: number; span: Span };

/** Parse `[proc,scale_by_playercount]`: `scale(sub(D, min(playercount, C)), D, base)`. */
function playerScale(ctx: Ctx): PlayerScale | Gap {
    const block = uniqueBlock(ctx, 'proc', 'scale_by_playercount');
    if (!block) return gap('scale-proc-missing');
    const { flat } = flatten(block);
    const cap = flat.map(({ stmt }) => /^def_int \$playercount = min\(playercount, (\d+)\)$/.exec(stmt.text)).find(Boolean);
    const scaled = flat.map(({ stmt }) => /^return \(scale\(sub\((\d+), \$playercount\), (\d+), \$base\)\)$/.exec(stmt.text)).find(Boolean);
    if (!cap || !scaled || scaled[1] !== scaled[2] || flat.length !== 2) return gap('scale-proc-shape', blockSpan(block));
    return { cap: Number(cap[1]), denominator: Number(scaled[1]), span: blockSpan(block) };
}

/** Bounds of `scale(D - min(players, C), D, base)` in ticks. Engine `scale(a,b,c)` is `trunc(a*c/b)` (int32). */
function scaleBounds(scale: PlayerScale, base: number) {
    return { min: Math.trunc(((scale.denominator - scale.cap) * base) / scale.denominator), max: base };
}

// ---- tools -----------------------------------------------------------------------------------------------

/** Attack wield gate of one tool from its `opheld2` handler. `gap` when the handler is not a recognised gate form. */
function wieldGate(ctx: Ctx, alias: string): { gate: GateWire | null } | { gap: Gap } {
    const category = ctx.idx.obj.get(alias)?.props.get('category')?.[0];
    const headers = [...(ctx.idx.headers.get(`opheld2,${alias}`) ?? []), ...(category ? ctx.idx.headers.get(`opheld2,_${category}`) ?? [] : [])];
    if (headers.length === 0) return { gate: null };
    if (headers.length > 1) return { gap: gap('wield-handler-ambiguous', ...headers.map((header) => header.span)) };
    const header = headers[0];
    const block = ctx.idx.blocks(header.span.file).find((each) => each.span.first === header.span.first)!;
    const text = codeText(block);
    const attack = /^@levelrequire_attack\((\d+), last_slot\);$/.exec(text);
    if (attack) return { gate: { skill: SKILL.attack, level: Number(attack[1]) } };
    if (text === '@tutorial_island_equip(last_slot);') {
        // The tutorial-only guard is ignored on purpose: it can only hold before Tutorial Island ends. The gate that
        // remains for every post-tutorial account is the Attack minimum on its final line.
        const label = uniqueBlock(ctx, 'label', 'tutorial_island_equip');
        const { nodes } = label ? flatten(label) : { nodes: [] as Node[] };
        const last = nodes.filter((node) => node.kind === 'stmt').map((node) => (node as Stmt).text).filter((each) => each.startsWith('@levelrequire_attack('));
        const final = last.length === 1 ? /^@levelrequire_attack\((\d+), \$slot\)$/.exec(last[0]) : null;
        if (final) return { gate: { skill: SKILL.attack, level: Number(final[1]) } };
    }
    return { gap: gap('wield-handler-unrecognized', header.span) };
}

/** Best-first tool aliases from the checker proc's `if (... return(X))` chain. */
function checkerTools(ctx: Ctx, procName: string, condition: RegExp): { aliases: string[]; span: Span; ok: boolean } {
    const block = uniqueBlock(ctx, 'proc', procName);
    if (!block) return { aliases: [], span: { file: '', first: 0, last: 0 }, ok: false };
    const nodes = parseBody(block.body);
    const aliases: string[] = [];
    let ok = true;
    for (const node of nodes) {
        if (node.kind === 'if') {
            const match = condition.exec(node.cond);
            const ret = node.then.length === 1 && node.then[0].kind === 'stmt' ? /^return\s*\((\w+)\)$/.exec(node.then[0].text) : null;
            if (match && ret && ret[1] === match[1] && node.els.length === 0) aliases.push(match[1]);
            else if (!node.cond.startsWith('$print_mes')) ok = false;
        }
    }
    return { aliases, span: blockSpan(block), ok: ok && aliases.length > 0 };
}

function toolRows(ctx: Ctx, aliases: string[], use: (alias: string) => GateWire | null, sourceGaps: Gap[]): Know<ToolWire[]> {
    const gaps = [...sourceGaps];
    const rows = aliases.map((alias) => {
        const wield = wieldGate(ctx, alias);
        if ('gap' in wield) gaps.push(wield.gap);
        return { item: ctx.entities.id('obj', alias, `tool ${alias}`), use_gate: use(alias), wield_gate: 'gate' in wield ? wield.gate : null };
    });
    return partial(rows, gaps);
}

function pickaxes(ctx: Ctx): Know<ToolWire[]> {
    const found = checkerTools(ctx, 'pickaxe_checker', /^\$lvl0 >= oc_param\((\w+), levelrequire\) & \(\$obj1 = \1 \| inv_total\(inv, \1\) > 0\)$/);
    if (!found.ok) return unknown('pickaxe-checker-unrecognized', found.span.file ? [spanRef(found.span)] : []);
    return toolRows(ctx, found.aliases, (alias) => {
        const level = ctx.idx.obj.get(alias)?.params.get('levelrequire')?.[0];
        if (level === undefined) throw new Error(`pickaxe ${alias}: missing levelrequire param`);
        return { skill: SKILL.mining, level: Number(level) };
    }, []);
}

function axes(ctx: Ctx): Know<ToolWire[]> {
    const found = checkerTools(ctx, 'woodcutting_axe_checker', /^\(\$obj1 = (\w+) \| inv_total\(inv, \1\) > 0\)$/);
    if (!found.ok) return unknown('axe-checker-unrecognized', found.span.file ? [spanRef(found.span)] : []);
    // The checker has no level comparison: the Woodcutting use gate is absent, not unknown.
    return toolRows(ctx, found.aliases, () => null, []);
}

// ---- mining (M-215 classification) ------------------------------------------------------------------

const MINE_DBROW = 'scripts/skill_mining/configs/mine.dbrow';
const MINING_SCRIPT = 'scripts/skill_mining/scripts/mining.rs2';
const GAS_SCRIPT = 'scripts/macro events/scripts/mining/macro_event_gas.rs2';
const MINING_COLUMNS = ['rock', 'ore_name', 'rock_output', 'rock_level', 'rock_exp', 'rock_successchance', 'rock_respawnrate'];

type RockClass = 'resource' | 'depleted' | 'hazard' | 'unclassified';
type Rock = { alias: string; span: Span; class: RockClass; slot: number | null; label: string | null; gap: Gap | null };

/** What one standard mining label (`get_ore_*`) is proven to do, by exact statement anchors. */
type MiningLabel = { block: Rs2Block; depletes: boolean; scaledRespawn: boolean; gemRoll: boolean; dropTable: string | null; interceptProc: string | null; crestZone: Span | null; deletes: boolean };

const MINING_LABELS: Record<string, { anchors: (string | RegExp)[]; depletes: boolean }> = {
    get_ore_normal: {
        depletes: true,
        anchors: [
            'db_find(mining_table:rock, loc_type)',
            'def_int $respawn = ~scale_by_playercount(db_getfield($data, mining_table:rock_respawnrate, 0))',
            'loc_change(loc_param(next_loc_stage_mining), $respawn)',
            'def_namedobj $output = db_getfield($data, mining_table:rock_output, 0)',
            'inv_add(inv, $output, 1)',
            'stat_advance(mining, db_getfield($data, mining_table:rock_exp, 0))',
        ],
    },
    get_ore_fast: {
        depletes: true,
        anchors: [
            'db_find(mining_table:rock, loc_type)',
            'def_int $respawn = ~scale_by_playercount(db_getfield($data, mining_table:rock_respawnrate, 0))',
            'loc_change(loc_param(next_loc_stage_mining), $respawn)',
            'inv_add(inv, db_getfield($data, mining_table:rock_output, 0), 1)',
            'stat_advance(mining, db_getfield($data, mining_table:rock_exp, 0))',
        ],
    },
    get_ore_essence: {
        depletes: false,
        anchors: [
            'db_find(mining_table:rock, loc_type)',
            'inv_add(inv, db_getfield($data, mining_table:rock_output, 0), 1)',
            'stat_advance(mining, db_getfield($data, mining_table:rock_exp, 0))',
        ],
    },
    get_ore_gem_rock: {
        depletes: true,
        anchors: [
            'db_find(mining_table:rock, loc_type)',
            'def_int $respawn = ~scale_by_playercount(db_getfield($data, mining_table:rock_respawnrate, 0))',
            'loc_change(loc_param(next_loc_stage_mining), $respawn)',
            '$gem, $count = ~roll_on_drop_table(gem_rock_table)',
            'inv_add(inv, $gem, 1)',
            'stat_advance(mining, db_getfield($data, mining_table:rock_exp, 0))',
        ],
    },
};

function miningLabel(ctx: Ctx, name: string): MiningLabel | Gap {
    const block = uniqueBlock(ctx, 'label', name);
    const spec = MINING_LABELS[name];
    if (!block || !spec) return gap('mining-label-missing');
    const { flat, nodes } = flatten(block);
    const conds = conditions(nodes);
    if (!spec.anchors.every((anchor) => hasStmt(flat, anchor)) || hasStmt(flat, /^loc_change\(/) !== spec.depletes) return gap('mining-label-shape', blockSpan(block));
    const crest = conds.find((cond) => /^inzone\(\^crest_perfect_mine_lower_bound, \^crest_perfect_mine_upper_bound, coord\) = true$/.test(cond));
    const intercept = conds.map((cond) => /~(magnus_intercept_ore)\(/.exec(cond)).find(Boolean);
    return {
        block,
        depletes: spec.depletes,
        scaledRespawn: spec.depletes,
        gemRoll: conds.includes('random($chance) = ^true') && hasStmt(flat, 'def_namedobj $gem = ~mining_gem_table'),
        dropTable: name === 'get_ore_gem_rock' ? 'gem_rock_table' : null,
        interceptProc: intercept ? intercept[1] : null,
        crestZone: crest ? blockSpan(block) : null,
        deletes: hasStmt(flat, /^inv_del|^inv_delslot|^inv_setslot/),
    };
}

/** Standard dispatch: which label does `oploc1` on this loc (by alias, else by category) run, and where is the header. */
function locDispatch(ctx: Ctx, alias: string, category: string | undefined) {
    const aliasHeaders = [1, 2, 3, 4, 5].flatMap((slot) => ctx.idx.headers.get(`oploc${slot},${alias}`) ?? []);
    const categoryHeaders = category ? [1, 2, 3, 4, 5].flatMap((slot) => ctx.idx.headers.get(`oploc${slot},_${category}`) ?? []) : [];
    return { aliasHeaders, categoryHeaders };
}

function firstSwingLabel(ctx: Ctx, header: { span: Span }): string | null {
    const block = ctx.idx.blocks(header.span.file).find((each) => each.span.first === header.span.first);
    const text = block ? codeText(block) : '';
    const match = /^@mining_firstswing\((get_ore_\w+)\);$/.exec(text);
    return match ? match[1] : null;
}

/** A custom-handled rock whose Mine handler provably yields one dbrow ore: ore group and handler span. */
type CustomOre = { ore: string; handler: Span };

type MiningModel = {
    rows: DbRow[];
    rocks: Map<string, Rock>;
    labels: Map<string, MiningLabel | Gap>;
    population: string[];
    /** M-215: custom-handler rocks promoted to resource by their proven ore product. */
    customOre: Map<string, CustomOre>;
};

/** dbrow rock outputs mapped to the ore groups that yield them (M-215 custom-rock join key). */
function oreOutputs(rows: DbRow[]): Map<string, string[]> {
    const out = new Map<string, string[]>();
    for (const row of rows) {
        const ore = row.data.get('ore_name')?.[0]?.values[0];
        const output = row.data.get('rock_output')?.[0]?.values[0];
        if (!ore || !output) continue;
        const ores = out.get(output) ?? out.set(output, []).get(output)!;
        if (!ores.includes(ore)) ores.push(ore);
    }
    return out;
}

const CUSTOM_YIELD = /^inv_add\(inv, ([A-Za-z0-9_]+), 1\)$/;
/** Any inventory grant in a handler: promotion requires exactly one of these in total. */
const ANY_GRANT = /^inv_add\s*\(/;

/**
 * M-215: prove a custom-handled rock's ore from its own Mine handler. Promotes only when the rock's
 * Mine slot is handled by exactly one custom `oploc` (no standard dispatch on that slot), and that
 * handler contains exactly one `inv_add` in total — a unit grant `inv_add(inv, ITEM, 1)` where ITEM
 * is the `rock_output` of exactly one `mine.dbrow` ore group. Repeated identical grants, extra
 * non-unit grants, or any other additional grant fails closed to unclassified, never guessed.
 */
function customOreTarget(ctx: Ctx, slot: number | null, aliasHeaders: Rs2Header[], categoryHeaders: Rs2Header[], outputs: Map<string, string[]>): CustomOre | null {
    if (slot === null) return null;
    const mine = `oploc${slot}`;
    const custom = aliasHeaders.filter((header) => header.kind === mine && !header.span.file.startsWith('scripts/skill_mining/'));
    if (custom.length !== 1) return null;
    if ([...aliasHeaders, ...categoryHeaders].some((header) => header.kind === mine && header.span.file === MINING_SCRIPT)) return null;
    const block = ctx.idx.blocks(custom[0].span.file).find((each) => each.span.first === custom[0].span.first);
    if (!block) return null;
    const { flat } = flatten(block);
    let grants = 0;
    const yields = new Set<string>();
    for (const { stmt } of flat) {
        if (!ANY_GRANT.test(stmt.text)) continue;
        grants += 1;
        const match = CUSTOM_YIELD.exec(stmt.text);
        if (match) yields.add(match[1]);
    }
    if (grants !== 1 || yields.size !== 1) return null;
    const ores = outputs.get([...yields][0]) ?? [];
    if (ores.length !== 1) return null;
    return { ore: ores[0], handler: custom[0].span };
}

function classifyRocks(ctx: Ctx, rows: DbRow[]): MiningModel {
    const tableMembers = new Set(rows.flatMap((row) => (row.data.get('rock') ?? []).map((data) => data.values[0])));
    const population = new Set<string>(tableMembers);
    for (const section of ctx.idx.loc.all()) {
        const ops = [...section.props].filter(([key]) => /^op[1-5]$/.test(key)).flatMap(([, values]) => values);
        const category = section.props.get('category')?.[0] ?? '';
        if (ops.some((op) => op.toLowerCase() === 'mine') || category.startsWith('mining_rock_') || ['next_loc_stage_mining', 'mining_rock_empty', 'macro_gas'].some((key) => section.params.has(key))) population.add(section.name);
    }
    for (const alias of [...population]) {
        const def = ctx.idx.loc.get(alias);
        for (const key of ['next_loc_stage_mining', 'macro_gas']) {
            const linked = def?.params.get(key)?.[0];
            if (linked) population.add(linked);
        }
    }
    const labels = new Map<string, MiningLabel | Gap>();
    const label = (name: string) => labels.get(name) ?? labels.set(name, miningLabel(ctx, name)).get(name)!;
    const gasHeader = (ctx.idx.headers.get('oploc1,_mining_rock_macro_gas') ?? []).find((header) => header.span.file === GAS_SCRIPT);
    const rocks = new Map<string, Rock>();
    const customOre = new Map<string, CustomOre>();
    const outputs = oreOutputs(rows);
    for (const alias of [...population].sort()) {
        const def = ctx.idx.loc.get(alias);
        if (!def) {
            rocks.set(alias, { alias, span: line('', 0), class: 'unclassified', slot: null, label: null, gap: gap('no-loc-config') });
            continue;
        }
        const category = def.props.get('category')?.[0];
        const slot = opSlot(def, 'mine');
        const { aliasHeaders, categoryHeaders } = locDispatch(ctx, alias, category);
        const custom = aliasHeaders.filter((header) => !header.span.file.startsWith('scripts/skill_mining/'));
        let result: Rock;
        if (category === 'mining_rock_macro_gas' && gasHeader && categoryHeaders.some((header) => header.span.file === GAS_SCRIPT) && custom.length === 0) {
            result = { alias, span: def.span, class: 'hazard', slot, label: null, gap: null };
        } else if (def.params.get('mining_rock_empty')?.[0] === '1' && custom.length === 0) {
            result = { alias, span: def.span, class: 'depleted', slot, label: null, gap: null };
        } else if (custom.length > 0) {
            const derived = customOreTarget(ctx, slot, aliasHeaders, categoryHeaders, outputs);
            if (derived) {
                customOre.set(alias, derived);
                result = { alias, span: def.span, class: 'resource', slot, label: null, gap: null };
            } else {
                result = { alias, span: def.span, class: 'unclassified', slot, label: null, gap: gap('custom-handler', def.span, ...custom.map((header) => header.span)) };
            }
        } else if (!tableMembers.has(alias)) {
            // M-215 R2/R3: no alias or category handler anywhere means the engine runs its
            // default no-op, so the loc cannot mine at all. (A global handler would have
            // failed closed at index time, so reaching here proves there is none.) It is not
            // a resource and not an unknown one either: exclude it from the mining catalogue
            // entirely, never a method target, rock fact, or coverage row.
            if (aliasHeaders.length + categoryHeaders.length === 0) {
                population.delete(alias);
                continue;
            }
            result = { alias, span: def.span, class: 'unclassified', slot, label: null, gap: gap('not-in-mining-table', def.span) };
        } else {
            const skill = [...aliasHeaders, ...categoryHeaders].filter((header) => header.span.file === MINING_SCRIPT && header.kind === 'oploc1');
            const name = skill.length === 1 ? firstSwingLabel(ctx, skill[0]) : null;
            const parsed = name ? label(name) : null;
            result = parsed && !('code' in parsed)
                ? { alias, span: def.span, class: 'resource', slot, label: name, gap: null }
                : { alias, span: def.span, class: 'unclassified', slot, label: null, gap: gap('mining-handler-unrecognized', def.span, ...skill.map((header) => header.span)) };
        }
        rocks.set(alias, result.class !== 'unclassified' && result.slot === null ? { ...result, class: 'unclassified', gap: gap('no-mine-op', def.span) } : result);
    }
    return { rows, rocks, labels, population: [...population].sort(), customOre };
}

// ---- assembly helpers ----------------------------------------------------------------------------------------

const CLASS_ORDER: Record<TargetClass, number> = { resource: 0, hazard: 1, depleted: 2, unclassified: 3 };
const byTarget = (a: TargetWire, b: TargetWire) => CLASS_ORDER[a.class] - CLASS_ORDER[b.class] || a.kind.localeCompare(b.kind) || a.id - b.id;

/** Respawn fact of a depleting resource: raw content value with its source-derived scale bounds. */
function respawnOf(scale: PlayerScale | Gap, raw: number, source: Span, rule: string, bounds?: { min: number; max: number }, extra: Span[] = []): RespawnWire {
    if ('code' in scale) return { raw, scale: unknown(scale.code, scale.sources), source: spanRef(source) };
    const range = bounds ?? scaleBounds(scale, raw);
    return { raw, scale: known({ rule, min_ticks: range.min, max_ticks: range.max, sources: [scale.span, ...extra].map(spanRef) }), source: spanRef(source) };
}


/** First `opN` slot whose label equals `label` (ASCII case-insensitive), or null. */
function opSlot(def: Section, label: string): number | null {
    for (let slot = 1; slot <= 5; slot += 1) if (def.props.get(`op${slot}`)?.[0]?.toLowerCase() === label) return slot;
    return null;
}

// ---- zones (content-defined regions where a method's yield is intercepted, substituted or gated) ----------

type Zone = { level: number; min_x: number; min_z: number; max_x: number; max_z: number };
const INZONE = /^inzone\(\^(\w+), \^(\w+), coord\) = true$/;

function zoneBox(ctx: Ctx, lower: string, upper: string): { zone: Zone; sources: Span[] } | null {
    const a = ctx.idx.constants.get(lower);
    const b = ctx.idx.constants.get(upper);
    if (!a || !b) return null;
    const p = parseCoord(a.value, lower);
    const q = parseCoord(b.value, upper);
    if (p.plane !== q.plane) return null;
    return { zone: { level: p.plane, min_x: Math.min(p.x, q.x), min_z: Math.min(p.z, q.z), max_x: Math.max(p.x, q.x), max_z: Math.max(p.z, q.z) }, sources: [a.span, b.span] };
}

/** Collects zone rules; one rule per zone and effect, listing every method it applies to and every source. */
class Zones {
    private readonly rules = new Map<string, ZoneWire>();
    add(method: string, effect: ZoneEffect, zone: Zone, sources: Span[]) {
        const key = `${effect}\0${zone.level}:${zone.min_x},${zone.min_z},${zone.max_x},${zone.max_z}`;
        const rule = this.rules.get(key) ?? this.rules.set(key, { methods: [], ...zone, effect, sources: [] }).get(key)!;
        if (!rule.methods.includes(method)) rule.methods.push(method);
        for (const ref of sources.map(spanRef)) if (!rule.sources.includes(ref)) rule.sources.push(ref);
    }
    list() {
        return [...this.rules.values()].map((rule) => ({ ...rule, methods: [...rule.methods].sort() })).sort((a, b) => a.effect.localeCompare(b.effect) || a.sources.join().localeCompare(b.sources.join()));
    }
}

/** Zone read from an intercept proc: `if (inzone(^A, ^B, coord) = true) { ... return (true); }`. */
function interceptZone(ctx: Ctx, proc: string): { zone: Zone; sources: Span[] } | Gap {
    const block = uniqueBlock(ctx, 'proc', proc);
    if (!block) return gap('intercept-proc-missing');
    const nodes = parseBody(block.body);
    const guard = nodes.find((node) => node.kind === 'if' && INZONE.test(node.cond));
    const yields = guard && guard.kind === 'if' && JSON.stringify(guard.then).includes('"text":"return (true)"');
    const match = guard && guard.kind === 'if' ? INZONE.exec(guard.cond) : null;
    const box = match && yields ? zoneBox(ctx, match[1], match[2]) : null;
    if (!box) return gap('intercept-proc-shape', blockSpan(block));
    return { zone: box.zone, sources: [blockSpan(block), ...box.sources] };
}

/** Zone-guarded early returns anywhere in a block (`if (inzone(...)) { ... return; }`). */
function zoneGates(ctx: Ctx, nodes: Node[], into: { zone: Zone; sources: Span[] }[] = [], file = ''): { zone: Zone; sources: Span[] }[] {
    for (const node of nodes) {
        if (node.kind !== 'if') continue;
        const match = INZONE.exec(node.cond);
        if (match && JSON.stringify(node.then).includes('"text":"return')) {
            const box = zoneBox(ctx, match[1], match[2]);
            if (box) into.push({ zone: box.zone, sources: [line(file, node.line), ...box.sources] });
        }
        zoneGates(ctx, node.then, into, file);
        zoneGates(ctx, node.els, into, file);
    }
    return into;
}

// ---- mining ------------------------------------------------------------------------------------------------

const GEM_DBROW = 'scripts/skill_mining/configs/gem_rock_table.dbrow';

type MiningOutput = { methods: MethodWire[]; loose: LooseWire[]; model: MiningModel };

/** Named objects returned by the content's `mining_gem_table` proc, joined through obj.pack. */
function miningGemIds(ctx: Ctx): number[] {
    const block = uniqueBlock(ctx, 'proc', 'mining_gem_table');
    if (!block) throw new Error(`${MINING_SCRIPT}: expected one mining_gem_table proc`);
    const outputs = flatten(block).flat.filter(({ stmt }) => /^return\b/.test(stmt.text));
    if (outputs.length === 0) throw new Error(`${MINING_SCRIPT}:${block.span.first}: mining_gem_table has no return statements`);
    const ids = new Set<number>();
    for (const { stmt } of outputs) {
        const returned = /^return\s*\(\s*([A-Za-z0-9_]+)\s*\)\s*;?$/.exec(stmt.text);
        if (!returned) throw new Error(`${MINING_SCRIPT}:${stmt.line}: unrecognized mining_gem_table output ${stmt.text}`);
        if (returned[1] !== 'null') ids.add(ctx.entities.id('obj', returned[1], 'mining_gem_table'));
    }
    if (ids.size === 0) throw new Error(`${MINING_SCRIPT}:${block.span.first}: mining_gem_table has no named object outputs`);
    return [...ids].sort((a, b) => a - b);
}

function extractMining(ctx: Ctx, pick: Know<ToolWire[]>, scale: PlayerScale | Gap, zones: Zones): MiningOutput {
    const dbtable = 'scripts/skill_mining/configs/mine.dbtable';
    for (const column of MINING_COLUMNS) if (!new RegExp(`^column=${column},`, 'm').test(ctx.idx.text(dbtable))) throw new Error(`${dbtable}: missing selected column ${column}`);
    const rows = parseDbRows(MINE_DBROW, ctx.idx.text(MINE_DBROW)).filter((row) => row.table === 'mining_table');
    if (rows.length === 0) throw new Error(`${MINE_DBROW}: no mining_table rows`);
    const model = classifyRocks(ctx, rows);
    const gemRow = parseDbRows(GEM_DBROW, ctx.idx.text(GEM_DBROW)).find((row) => row.name === 'gem_rock_table' && row.table === 'drop_table');
    const firstSwing = uniqueBlock(ctx, 'label', 'mining_firstswing');
    const groups = new Map<string, DbRow[]>();
    for (const row of rows) {
        const ore = row.data.get('ore_name')?.[0]?.values[0];
        if (!ore) throw new Error(`${row.name}: missing ore_name`);
        (groups.get(ore) ?? groups.set(ore, []).get(ore)!).push(row);
    }
    const attached = new Set<string>();
    const methods: MethodWire[] = [];
    for (const [ore, group] of groups) {
        const id = `mining.${ore}`;
        const targets = new Map<string, TargetWire>();
        const targetGaps: Gap[] = [];
        const labels = new Set<string>();
        const resourceRocks: Rock[] = [];
        const requirementGaps: Gap[] = [];
        const productGaps: Gap[] = [];
        const add = (rock: Rock, respawn: Know<RespawnWire | null>) => {
            attached.add(rock.alias);
            if (!targets.has(rock.alias)) targets.set(rock.alias, { kind: 'loc', id: ctx.entities.id('loc', rock.alias, id), op: rock.slot!, class: rock.class, respawn });
        };
        for (const row of group) {
            const rate = row.data.get('rock_respawnrate')?.[0];
            for (const data of row.data.get('rock') ?? []) {
                if (model.customOre.has(data.values[0])) continue;
                const rock = model.rocks.get(data.values[0])!;
                if (rock.class === 'unclassified') continue;
                const def = ctx.idx.loc.get(rock.alias)!;
                if (rock.class !== 'resource') {
                    add(rock, known(null));
                    continue;
                }
                const label = model.labels.get(rock.label!) as MiningLabel;
                labels.add(rock.label!);
                resourceRocks.push(rock);
                const next = def.params.get('next_loc_stage_mining')?.[0];
                let respawn: Know<RespawnWire | null>;
                if (!label.depletes) respawn = known(null);
                else if (!next) respawn = unknown('missing-next-loc-stage', [spanRef(rock.span)]);
                else if (!rate) respawn = unknown('respawn-rate-missing', [spanRef(row.span)]);
                else respawn = known(respawnOf(scale, Number(rate.values[0]), line(MINE_DBROW, rate.line), 'scale_by_playercount'));
                add(rock, respawn);
                for (const linked of [next, def.params.get('macro_gas')?.[0]]) {
                    if (!linked) continue;
                    const other = model.rocks.get(linked);
                    if (other?.class === 'resource') continue;
                    if (!other || other.class === 'unclassified') targetGaps.push(other?.gap ?? gap('linked-rock-unclassified', rock.span));
                    else add(other, known(null));
                }
            }
        }
        for (const [alias, derived] of model.customOre) {
            if (derived.ore !== ore) continue;
            const rock = model.rocks.get(alias)!;
            if (rock.class !== 'resource' || attached.has(alias)) continue;
            const respawn: Know<RespawnWire | null> = unknown('custom-deplete', [spanRef(rock.span), spanRef(derived.handler)]);
            add(rock, respawn);
            resourceRocks.push(rock);
            targetGaps.push(gap('custom-handler-target', derived.handler));
        }
        const resources = [...targets.values()].filter((target) => target.class === 'resource');
        const levels = new Set(group.map((row) => row.data.get('rock_level')?.[0]?.values[0]));
        const outputs = new Set(group.map((row) => row.data.get('rock_output')?.[0]?.values[0]));
        const first = group[0];
        const levelData = first.data.get('rock_level')?.[0];
        const level = levelData ? Number(levelData.values[0]) : NaN;
        if (levels.size !== 1 || Number.isNaN(level)) requirementGaps.push(gap('rows-disagree-rock-level', ...group.map((row) => row.span)));
        let products: Know<YieldWire[]>;
        if (outputs.size === 1 && [...outputs][0] !== undefined) {
            products = known([{ item: ctx.entities.id('obj', [...outputs][0]!, `${id} output`), level }]);
        } else if (outputs.size === 1 && labels.has('get_ore_gem_rock') && gemRow) {
            products = known((gemRow.data.get('drop') ?? []).map((drop) => ({ item: ctx.entities.id('obj', drop.values[0], `${id} gem drop`), level })));
        } else products = unknown('rock-output-unresolved', group.map((row) => spanRef(row.span)));
        if (products.state === 'known' && [...labels].some((name) => (model.labels.get(name) as MiningLabel).gemRoll)) {
            const gemBlock = uniqueBlock(ctx, 'proc', 'mining_gem_table');
            const rollSpans = [...labels].map((name) => blockSpan((model.labels.get(name) as MiningLabel).block));
            productGaps.push(gap('incidental-gem-roll', ...rollSpans.slice(0, 1), ...(gemBlock ? [blockSpan(gemBlock)] : [])));
        }
        const consumeGaps = [...labels].filter((name) => (model.labels.get(name) as MiningLabel).deletes).map((name) => gap('inventory-effect', blockSpan((model.labels.get(name) as MiningLabel).block)));
        const first_rock = resourceRocks[0];
        const opLabel = first_rock ? ctx.idx.loc.get(first_rock.alias)!.props.get(`op${first_rock.slot}`)?.[0] : undefined;
        const zoneMethod = (effect: ZoneEffect, found: { zone: Zone; sources: Span[] } | Gap) => {
            if ('zone' in found) zones.add(id, effect, found.zone, found.sources);
            else requirementGaps.push(found);
        };
        for (const name of labels) {
            const label = model.labels.get(name) as MiningLabel;
            if (label.interceptProc) zoneMethod('yield-intercepted', interceptZone(ctx, `${label.interceptProc}`));
            if (label.crestZone) {
                const box = zoneBox(ctx, 'crest_perfect_mine_lower_bound', 'crest_perfect_mine_upper_bound');
                zoneMethod('product-substituted', box ? { zone: box.zone, sources: [label.crestZone, ...box.sources] } : gap('zone-constant-missing', label.crestZone));
            }
        }
        if (firstSwing && resourceRocks.length > 0) {
            for (const gate of zoneGates(ctx, parseBody(firstSwing.body), [], firstSwing.span.file)) zones.add(id, 'state-gated', gate.zone, gate.sources);
        }
        methods.push({
            id,
            skill: 'mining',
            resources: [ore],
            op: first_rock && opLabel ? { slot: first_rock.slot!, label: opLabel } : null,
            targets: resources.length === 0 ? unknown('no-resource-target', group.map((row) => spanRef(row.span))) : partial([...targets.values()].sort(byTarget), targetGaps),
            products: productGaps.length > 0 && products.state === 'known' ? partial(products.value, productGaps) : products,
            tools: pick,
            consumes: labels.size === 0 ? unknown('no-mining-handler', group.map((row) => spanRef(row.span))) : partial([], consumeGaps),
            requirements: Number.isNaN(level) ? unknown('rock-level-missing', group.map((row) => spanRef(row.span))) : partial([{ id: `${id}.level`, source: spanRef(line(MINE_DBROW, levelData!.line)), kind: 'skill', skill: SKILL.mining, level }], requirementGaps),
            spots: resources.length === 0 ? unknown('no-resource-target', group.map((row) => spanRef(row.span))) : known(null),
            sources: group.map((row) => spanRef(row.span)),
        });
    }
    const loose: LooseWire[] = [];
    for (const alias of model.population) {
        if (attached.has(alias)) continue;
        const rock = model.rocks.get(alias)!;
        const id = ctx.entities.id('loc', alias, `loose rock ${alias}`);
        if (rock.class === 'depleted') loose.push({ skill: 'mining', kind: 'loc', id, class: 'depleted', gap: null });
        else loose.push({ skill: 'mining', kind: 'loc', id, class: 'unclassified', gap: rock.gap ?? gap('not-attached', rock.span) });
    }
    return { methods, loose, model };
}

// ---- woodcutting -------------------------------------------------------------------------------------------

const TREES_DBROW = 'scripts/skill_woodcutting/configs/trees.dbrow';
const WOODCUT_SCRIPT = 'scripts/skill_woodcutting/scripts/woodcut.rs2';
const TREE_COLUMNS = ['levelrequired', 'tree', 'productexp', 'product', 'respawnrate'];

type WoodModel = {
    /** `respawnrate=0` rows: base ticks are `add(base, random(spread))`, never depleting by chance. */
    zero: { base: number; spread: number; span: Span };
    membersGate: boolean;
    intercept: string | null;
    gates: { zone: Zone; sources: Span[] }[];
    deletes: boolean;
};

/** Verify the `woodcut.rs2` label shapes the extraction relies on; a changed shape is a gap, not a guess. */
function woodModel(ctx: Ctx): WoodModel | Gap {
    const attempt = uniqueBlock(ctx, 'label', 'attempt_cut_tree');
    const cut = uniqueBlock(ctx, 'label', 'cut_tree');
    const logs = uniqueBlock(ctx, 'label', 'get_logs');
    const dispatch = (ctx.idx.headers.get('oploc1,_tree') ?? []).filter((header) => header.span.file === WOODCUT_SCRIPT);
    if (!attempt || !cut || !logs || dispatch.length !== 1) return gap('woodcut-label-missing');
    const dispatchBlock = ctx.idx.blocks(WOODCUT_SCRIPT).find((block) => block.span.first === dispatch[0].span.first);
    if (!dispatchBlock || codeText(dispatchBlock) !== '@attempt_cut_tree;') return gap('woodcut-dispatch-shape', dispatch[0].span);
    const a = flatten(attempt);
    const l = flatten(logs);
    const c = flatten(cut);
    const anchored =
        ['db_find(woodcutting_trees:tree, loc_type)', 'def_namedobj $product = db_getfield($data, woodcutting_trees:product, 0)', 'def_int $level = db_getfield($data, woodcutting_trees:levelrequired, 0)'].every((each) => hasStmt(a.flat, each)) &&
        conditions(a.nodes).includes('stat(woodcutting) < $level') &&
        ['def_int $respawnrate = db_getfield($data, woodcutting_trees:respawnrate, 0)', 'def_int $deplete_chance = 8', 'inv_add(inv, $product, 1)', '$respawnrate = ~scale_by_playercount($respawnrate)', 'loc_change(loc_param(next_loc_stage), $respawnrate)'].every((each) => hasStmt(l.flat, each)) &&
        ['db_find(woodcutting_trees:tree, loc_type)', 'def_int $level = db_getfield($data, woodcutting_trees:levelrequired, 0)'].every((each) => hasStmt(c.flat, each));
    const zeroBranch = l.nodes.find((node) => node.kind === 'if' && node.cond === '$respawnrate = 0');
    const variance = zeroBranch && zeroBranch.kind === 'if' ? zeroBranch.then.map((node) => (node.kind === 'stmt' ? /^\$respawnrate = add\((\d+), random\((\d+)\)\)$/.exec(node.text) : null)).find(Boolean) : null;
    const alwaysDeplete = zeroBranch && zeroBranch.kind === 'if' && zeroBranch.then.some((node) => node.kind === 'stmt' && node.text === '$deplete_chance = 0');
    if (!anchored || !variance || !alwaysDeplete) return gap('woodcut-label-shape', blockSpan(logs));
    const intercept = conditions(l.nodes).map((cond) => /~(leif_intercept_wood)\(/.exec(cond)).find(Boolean);
    return {
        zero: { base: Number(variance[1]), spread: Number(variance[2]), span: blockSpan(logs) },
        membersGate: conditions(a.nodes).includes('oc_members($product) = true') && JSON.stringify(a.nodes).includes('map_members = ^false'),
        intercept: intercept ? intercept[1] : null,
        gates: zoneGates(ctx, a.nodes, [], WOODCUT_SCRIPT),
        deletes: [a, l, c].some((each) => hasStmt(each.flat, /^inv_del|^inv_delslot|^inv_setslot/)),
    };
}

function woodKey(table: string) {
    const suffix = '_tree_table';
    if (!table.endsWith(suffix) || table.length === suffix.length) throw new Error(`${table}: expected wood table key`);
    return table.slice(0, -suffix.length);
}

/** Why a tree from the dbrow is not a standard woodcutting target. */
function treeGap(ctx: Ctx, def: Section | undefined, category: string | undefined, aliasSpans: Span[], model: WoodModel | Gap): Gap {
    if (!def) return gap('no-loc-config');
    if (aliasSpans.length > 0) return gap('custom-handler', def.span, ...aliasSpans);
    if ('code' in model) return model;
    const custom = category ? [1, 2, 3, 4, 5].flatMap((slot) => ctx.idx.headers.get(`oploc${slot},_${category}`) ?? []) : [];
    return custom.length > 0 ? gap('custom-handler', def.span, ...custom.map((header) => header.span)) : gap('tree-category-unrecognized', def.span);
}

function extractWoodcutting(ctx: Ctx, axeCell: Know<ToolWire[]>, scale: PlayerScale | Gap, zones: Zones): { methods: MethodWire[]; loose: LooseWire[]; hazardNpcs: number[] } {
    const dbtable = 'scripts/skill_woodcutting/configs/trees.dbtable';
    for (const column of TREE_COLUMNS) if (!new RegExp(`^column=${column},`, 'm').test(ctx.idx.text(dbtable))) throw new Error(`${dbtable}: missing selected column ${column}`);
    const rows = parseDbRows(TREES_DBROW, ctx.idx.text(TREES_DBROW)).filter((row) => row.table === 'woodcutting_trees');
    if (rows.length === 0) throw new Error(`${TREES_DBROW}: no woodcutting_trees rows`);
    const model = woodModel(ctx);
    const methods: MethodWire[] = [];
    const loose: LooseWire[] = [];
    const hazardNpcs = new Set<number>();
    for (const row of rows) {
        const key = woodKey(row.name);
        const id = `woodcutting.${key}`;
        const levelData = row.data.get('levelrequired')?.[0];
        const productData = row.data.get('product')?.[0];
        const rateData = row.data.get('respawnrate')?.[0];
        if (!levelData || !productData || !rateData) throw new Error(`${row.name}: missing levelrequired, product or respawnrate`);
        const level = Number(levelData.values[0]);
        const rate = Number(rateData.values[0]);
        const targets = new Map<string, TargetWire>();
        const targetGaps: Gap[] = [];
        let slot = 0;
        let opLabel: string | undefined;
        for (const data of row.data.get('tree') ?? []) {
            const alias = data.values[0];
            const def = ctx.idx.loc.get(alias);
            const ent = def?.params.get('ent')?.[0];
            if (ent && ent !== 'null') hazardNpcs.add(ctx.entities.id('npc', ent, `${row.name} tree ent`));
            const locId = ctx.entities.id('loc', alias, row.name);
            const aliasHeaders = [1, 2, 3, 4, 5].flatMap((each) => ctx.idx.headers.get(`oploc${each},${alias}`) ?? []);
            const category = def?.props.get('category')?.[0];
            if ('code' in model || !def || category !== 'tree' || aliasHeaders.length > 0) {
                loose.push({ skill: 'woodcutting', kind: 'loc', id: locId, class: 'unclassified', gap: treeGap(ctx, def, category, aliasHeaders.map((header) => header.span), model) });
                continue;
            }
            const treeSlot = [1, 2, 3, 4, 5].find((each) => (ctx.idx.headers.get(`oploc${each},_tree`) ?? []).some((header) => header.span.file === WOODCUT_SCRIPT) && (def.props.get(`op${each}`)?.[0] ?? 'hidden').toLowerCase() !== 'hidden') ?? null;
            if (treeSlot === null) {
                loose.push({ skill: 'woodcutting', kind: 'loc', id: locId, class: 'unclassified', gap: gap('no-chop-op', def.span) });
                continue;
            }
            slot = slot || treeSlot;
            opLabel ??= def.props.get(`op${treeSlot}`)?.[0];
            const stump = def.params.get('next_loc_stage')?.[0];
            let respawn: Know<RespawnWire | null>;
            if (!stump) respawn = unknown('missing-next-loc-stage', [spanRef(def.span)]);
            else if (rate === 0) {
                const range = 'code' in scale ? undefined : { min: scaleBounds(scale, model.zero.base).min, max: model.zero.base + model.zero.spread - 1 };
                respawn = known(respawnOf(scale, rate, line(TREES_DBROW, rateData.line), 'zero-rate-tree-variance+scale_by_playercount', range, [model.zero.span]));
            } else respawn = known(respawnOf(scale, rate, line(TREES_DBROW, rateData.line), 'scale_by_playercount'));
            targets.set(alias, { kind: 'loc', id: locId, op: treeSlot, class: 'resource', respawn });
            if (stump) {
                const stumpDef = ctx.idx.loc.get(stump);
                const stumpId = ctx.entities.id('loc', stump, `${row.name} stump`);
                if (!stumpDef) targetGaps.push(gap('no-loc-config', def.span));
                else if (!targets.has(stump)) targets.set(stump, { kind: 'loc', id: stumpId, op: treeSlot, class: 'depleted', respawn: known(null) });
            }
        }
        const resources = [...targets.values()].filter((target) => target.class === 'resource');
        const sources = [row.span].map(spanRef);
        const productAlias = productData.values[0];
        const productObj = ctx.idx.obj.get(productAlias);
        const requirementGaps: Gap[] = [];
        const requirements: RequirementWire[] = [{ id: `${id}.level`, source: spanRef(line(TREES_DBROW, levelData.line)), kind: 'skill', skill: SKILL.woodcutting, level }];
        if (!('code' in model) && model.membersGate && productObj?.props.get('members')?.[0] === 'yes') requirements.push({ id: `${id}.members`, source: spanRef(row.span), kind: 'members' });
        if ('code' in model) requirementGaps.push(model);
        else {
            if (model.intercept) {
                const found = interceptZone(ctx, model.intercept);
                if ('zone' in found) zones.add(id, 'yield-intercepted', found.zone, found.sources);
                else requirementGaps.push(found);
            }
            for (const gate of model.gates) zones.add(id, 'state-gated', gate.zone, gate.sources);
        }
        const empty = resources.length === 0;
        const why = 'code' in model ? model : gap('no-resource-target', row.span);
        methods.push({
            id,
            skill: 'woodcutting',
            resources: [key],
            op: empty || !opLabel ? null : { slot, label: opLabel },
            targets: empty ? unknown(why.code, why.sources) : partial([...targets.values()].sort(byTarget), targetGaps),
            products: empty ? unknown(why.code, why.sources) : known([{ item: ctx.entities.id('obj', productAlias, `${id} product`), level }]),
            tools: empty ? unknown(why.code, why.sources) : axeCell,
            consumes: empty ? unknown(why.code, why.sources) : 'code' in model ? unknown(model.code, model.sources) : partial([], model.deletes ? [gap('inventory-effect', blockSpan(uniqueBlock(ctx, 'label', 'get_logs')!))] : []),
            requirements: empty ? unknown(why.code, why.sources) : partial(requirements, requirementGaps),
            spots: empty ? unknown(why.code, why.sources) : known(null),
            sources,
        });
    }
    return { methods, loose, hazardNpcs: [...hazardNpcs].sort((a, b) => a - b) };
}

// ---- fishing -------------------------------------------------------------------------------------------------

const FISHING_NPC = 'scripts/skill_fishing/configs/fishing.npc';
const FISHING_SCRIPTS = 'scripts/skill_fishing/scripts/';

const LEVEL_LT = /^stat\(fishing\) < (\d+)$/;
const LEVEL_GE = /^stat\(fishing\) >= (\d+)$/;
const LOCAL_LEVEL_LT = /^\$level < (\d+)$/;
const EQUIP_FALSE = /^~check_fish_equipment\((\w+)\) = false$/;
const CHECK_PROC_FALSE = /^~(\w+) = false$/;
const HELD_LT1 = /^inv_total\(inv, (\w+)\) < 1$/;
const IGNORED_GUARD = [
    /^inv_freespace\(inv\) (?:< 1|= 0)(?: & inv_total\(inv, \w+\) > 1)?$/,
    /^%action_delay (?:<|=) (?:map_clock|calc\(map_clock \+ \d+\))$/,
    /^random\(\d+\) = 0$/,
    /^npc_param\(is_whirlpool\) = \^true$/,
    /^stat_random\(fishing, \d+, \d+\) = true$/,
    /^\$fish2 ! null$/,
    /^\$bait ! null$/,
];
const BENIGN_STATEMENT = [
    /^anim\(/, /^mes\(/, /^~mesbox\(/, /^~objbox\(/, /^sound_synth\(/, /^p_delay\(/, /^p_aprange\(/, /^%action_delay = /, /^stat_advance\(/, /^~displaymessage\(/,
    /^def_\w+ \$\w+/, /^\$\w+ = /, /^return\b/, /^session_log\(/, /^~macro_whirlpool_attempt_take_equipment\(/,
];
const IGNORED_LABELS = new Set(['macro_event_fishing', 'fishing_wrong_spot_message']);

type Roll = { f1: string; f2: string | null; bait: string | null; k: number };
type Closure = {
    levels: { level: number; span: Span }[];
    tools: Set<string>;
    held: Set<string>;
    rolls: Roll[];
    direct: { item: string; k: number }[];
    usesRoll: 'fish_roll' | 'fish_roll_loc' | null;
    requirementGaps: Gap[];
    effectGaps: Gap[];
    gates: { zone: Zone; sources: Span[] }[];
    visited: number;
};

const nullable = (alias: string) => (alias === 'null' ? null : alias);

/**
 * Anchors of the shared roll procs: a changed shape makes every roll-based method partial. `intercepts` is true
 * when the proc hands the catch to the Miscellania fisherman (a revision-dependent, optional construct).
 */
function rollShape(ctx: Ctx, name: 'fish_roll' | 'fish_roll_loc'): { gap: Gap | null; intercepts: boolean } {
    const block = uniqueBlock(ctx, 'proc', name);
    if (!block) return { gap: gap('fish-roll-missing'), intercepts: false };
    const { flat, nodes } = flatten(block);
    const conds = conditions(nodes);
    const ok =
        hasStmt(flat, 'inv_del(inv, $bait, 1)') &&
        hasStmt(flat, 'inv_add(inv, $fish2, 1)') &&
        hasStmt(flat, 'inv_add(inv, $fish1, 1)') &&
        hasStmt(flat, 'stat_advance(fishing, struct_param($struct2, productexp))') &&
        hasStmt(flat, 'stat_advance(fishing, struct_param($struct1, productexp))') &&
        conds.includes('$fish2 ! null') &&
        conds.includes('$bait ! null') &&
        (name === 'fish_roll_loc' || conds.includes('npc_param(is_whirlpool) = ^true'));
    const intercepts = conds.some((cond) => cond.includes('~frodi_intercept_fish('));
    const otherProcs = flat.some(({ stmt }) => /^~(?!macro_whirlpool_attempt_take_equipment)\w+/.test(stmt.text) && !stmt.text.startsWith('~frodi_intercept_fish('));
    return { gap: ok && !otherProcs ? null : gap('fish-roll-shape', blockSpan(block)), intercepts };
}

/** `baitrequired` of a fishing tool's `fish_equipment_struct`, or null when the struct declares none. */
function equipmentBait(ctx: Ctx, alias: string): { bait: string | null; span: Span } | Gap {
    const struct = ctx.idx.obj.get(alias)?.params.get('fish_equipment_struct')?.[0];
    const section = struct ? ctx.idx.structs.get(struct) : undefined;
    if (!section) return gap('equipment-struct-missing', ...(ctx.idx.obj.get(alias) ? [ctx.idx.obj.get(alias)!.span] : []));
    return { bait: section.params.get('baitrequired')?.[0] ?? null, span: section.span };
}

function checkEquipmentShape(ctx: Ctx): Gap | null {
    const block = uniqueBlock(ctx, 'proc', 'check_fish_equipment');
    if (!block) return gap('check-equipment-missing');
    const { flat, nodes } = flatten(block);
    const conds = conditions(nodes);
    const ok =
        hasStmt(flat, 'def_struct $struct = oc_param($fish_equipment, fish_equipment_struct)') &&
        conds.includes('inv_total(inv, $fish_equipment) < 1') &&
        hasStmt(flat, 'def_namedobj $bait = struct_param($struct, baitrequired)') &&
        conds.includes('$bait ! null & inv_total(inv, $bait) < 1');
    return ok ? null : gap('check-equipment-shape', blockSpan(block));
}

/** Follow one method's handler blocks (labels, re-dispatch, recognised procs) and collect proven facts and gaps. */
function fishingClosure(ctx: Ctx, start: Rs2Block, redispatch: (slot: number) => Rs2Block | null): Closure {
    const closure: Closure = { levels: [], tools: new Set(), held: new Set(), rolls: [], direct: [], usesRoll: null, requirementGaps: [], effectGaps: [], gates: [], visited: 0 };
    const seen = new Set<string>();
    const queue: Rs2Block[] = [start];
    const enqueue = (block: Rs2Block | null) => {
        if (block) queue.push(block);
    };
    const walk = (nodes: Node[], path: PathCond[], floor: number, block: Rs2Block, local: boolean) => {
        let current = floor;
        nodes.forEach((node, index) => {
            if (node.kind === 'while') return walk(node.body, [...path, { cond: node.cond, polarity: true, line: node.line }], current, block, local);
            if (node.kind === 'stmt') return statement(node, nodes, index, path, current, block);
            if (isRandomEvent(node.cond) || INZONE.test(node.cond)) return;
            const here = [...path, { cond: node.cond, polarity: true, line: node.line }];
            if (returns(node.then)) {
                if (guard(here, node.cond, node.line, block, local)) current = Math.max(current, Number(LOCAL_LEVEL_LT.exec(node.cond)?.[1] ?? 0));
            }
            walk(node.then, here, current, block, local);
            walk(node.els, [...path, { cond: node.cond, polarity: false, line: node.line }], current, block, local);
        });
    };
    /** Classify a returning guard; true when it is a `$level` floor for the statements that follow it. */
    const guard = (path: PathCond[], cond: string, at: number, block: Rs2Block, local: boolean): boolean => {
        const span = line(block.span.file, at);
        if (path.some((each) => INZONE.test(each.cond))) return false;
        let match: RegExpExecArray | null;
        if ((match = LEVEL_LT.exec(cond))) closure.levels.push({ level: Number(match[1]), span });
        else if (LOCAL_LEVEL_LT.test(cond)) {
            if (local) return true;
            closure.requirementGaps.push(gap('custom-guard', span));
        }
        else if ((match = EQUIP_FALSE.exec(cond))) closure.tools.add(match[1]);
        else if ((match = HELD_LT1.exec(cond))) closure.held.add(match[1]);
        else if ((match = CHECK_PROC_FALSE.exec(cond)) && match[1] !== 'mm_wearing_greegree') enqueue(uniqueBlock(ctx, 'proc', match[1]));
        else if (cond === '~mm_wearing_greegree = true') closure.requirementGaps.push(gap('monkey-form-forbidden', span));
        else if (IGNORED_GUARD.some((pattern) => pattern.test(cond))) return false;
        else closure.requirementGaps.push(gap(cond.includes('%') ? 'varp-gate' : 'custom-guard', span));
        return false;
    };
    const statement = (stmt: Stmt, siblings: Node[], index: number, path: PathCond[], floor: number, block: Rs2Block) => {
        const text = stmt.text;
        const span = line(block.span.file, stmt.line);
        const k = Math.max(floor, ...path.filter((each) => each.polarity).map((each) => Number(LEVEL_GE.exec(each.cond)?.[1] ?? 0)));
        let match: RegExpExecArray | null;
        if ((match = /^~fish_roll\((\w+), (\w+), (\w+), (\w+)\)$/.exec(text))) {
            closure.usesRoll = 'fish_roll';
            closure.rolls.push({ f1: match[1], f2: nullable(match[2]), bait: nullable(match[4]), k });
        } else if ((match = /^~fish_roll_loc\((\w+), (\w+), (\w+)\)$/.exec(text))) {
            closure.usesRoll = 'fish_roll_loc';
            closure.rolls.push({ f1: match[1], f2: nullable(match[2]), bait: nullable(match[3]), k });
        } else if (text === '~fish_roll_big_net') enqueue(uniqueBlock(ctx, 'proc', 'fish_roll_big_net'));
        else if ((match = /^@(\w+)(?:\(.*\))?$/.exec(text))) {
            if (!IGNORED_LABELS.has(match[1])) enqueue(uniqueBlock(ctx, 'label', match[1]));
        } else if ((match = /^p_(?:opnpc|oploc)\((\d+)\)$/.exec(text))) enqueue(redispatch(Number(match[1])));
        else if ((match = /^inv_add\(inv, (\w+), \d+\)$/.exec(text))) {
            // A catch is the nearest `inv_add` before a `stat_advance(fishing, ...)` in the same list.
            const rewardAt = siblings.findIndex((each, at) => at > index && each.kind === 'stmt' && each.text.startsWith('stat_advance(fishing,'));
            const nearest = rewardAt < 0 ? -1 : siblings.map((each, at) => (at < rewardAt && each.kind === 'stmt' && each.text.startsWith('inv_add(inv,') ? at : -1)).reduce((best, at) => Math.max(best, at), -1);
            if (rewardAt >= 0 && nearest === index) closure.direct.push({ item: match[1], k });
            else closure.effectGaps.push(gap('inventory-effect', span));
        } else if (/^inv_(?:del|delslot|setslot|changeslot)\(/.test(text)) closure.effectGaps.push(gap('inventory-effect', span));
        else if (BENIGN_STATEMENT.some((pattern) => pattern.test(text))) return;
        else closure.effectGaps.push(gap(text.startsWith('~') ? `unmodelled-proc:${/^~\.?(\w+)/.exec(text)?.[1] ?? '?'}` : 'unrecognized-statement', span));
    };
    while (queue.length > 0) {
        const block = queue.shift()!;
        const key = `${block.span.file}:${block.span.first}`;
        if (seen.has(key)) continue;
        seen.add(key);
        closure.visited += 1;
        const nodes = parseBody(block.body);
        const defines = nodes.some((node) => node.kind === 'stmt' && node.text === 'def_int $level = stat(fishing)');
        walk(nodes, [], 0, block, defines);
        closure.gates.push(...zoneGates(ctx, nodes, [], block.span.file));
    }
    return closure;
}

type FishSubject = { kind: 'npc' | 'loc'; dispatch: string; id: string; members: { alias: string; def: Section | undefined }[]; hazards: string[] };

/** Slots (1-5) with a dispatch header for the subject. */
const dispatchSlots = (ctx: Ctx, kind: 'npc' | 'loc', name: string) => [1, 2, 3, 4, 5].filter((slot) => (ctx.idx.headers.get(`op${kind}${slot},${name}`) ?? []).length > 0);

function regionOf(ctx: Ctx, alias: string): Know<RegionWire | null> {
    const def = ctx.idx.npc.get(alias);
    const enumName = def?.params.get('fishing_movement_enum')?.[0];
    if (!def || !enumName) return known(null);
    const category = def.props.get('category')?.[0];
    const moves = (ctx.idx.headers.get(`ai_timer,${alias}`) ?? []).length + (category ? (ctx.idx.headers.get(`ai_timer,_${category}`) ?? []).length : 0);
    if (moves === 0) return known(null);
    const section = ctx.idx.enums.get(enumName);
    if (!section) return unknown('movement-enum-missing', [spanRef(def.span)]);
    const coords = (section.props.get('val') ?? []).map((value) => parseCoord(value.slice(value.indexOf(',') + 1), enumName));
    if (coords.length === 0 || new Set(coords.map((coord) => coord.plane)).size !== 1) return unknown('movement-enum-shape', [spanRef(section.span)]);
    return known({ min_x: Math.min(...coords.map((c) => c.x)), min_z: Math.min(...coords.map((c) => c.z)), max_x: Math.max(...coords.map((c) => c.x)), max_z: Math.max(...coords.map((c) => c.z)), level: coords[0].plane });
}

function extractFishing(ctx: Ctx, zones: Zones): { methods: MethodWire[]; loose: LooseWire[]; movements: MovementWire[]; hazardNpcs: number[] } {
    const npcs = ctx.idx.npc.all().filter((section) => section.span.file === FISHING_NPC);
    if (npcs.length === 0) throw new Error(`${FISHING_NPC}: no fishing npc types`);
    const subjects = new Map<string, FishSubject>();
    const loose: LooseWire[] = [];
    const hazardAliases = new Set<string>();
    const looseNpc = (alias: string, code: string, ...spans: Span[]) => loose.push({ skill: 'fishing', kind: 'npc', id: ctx.entities.id('npc', alias, `loose ${alias}`), class: 'unclassified', gap: gap(code, ...spans) });
    const join = (key: string, subject: FishSubject) => subjects.get(key) ?? subjects.set(key, subject).get(key)!;
    for (const section of npcs) {
        const alias = section.name;
        const category = section.props.get('category')?.[0];
        const aliasHeaders = [1, 2, 3, 4, 5].flatMap((slot) => ctx.idx.headers.get(`opnpc${slot},${alias}`) ?? []);
        if (category && dispatchSlots(ctx, 'npc', `_${category}`).length > 0 && aliasHeaders.length === 0) {
            join(`_${category}`, { kind: 'npc', dispatch: `_${category}`, id: category, members: [], hazards: [] }).members.push({ alias, def: section });
        } else if (category && dispatchSlots(ctx, 'npc', `_${category}`).length > 0) {
            looseNpc(alias, 'custom-handler', section.span, ...aliasHeaders.map((header) => header.span));
        } else if (aliasHeaders.length > 0 && aliasHeaders.every((header) => header.span.file.startsWith(FISHING_SCRIPTS))) {
            join(alias, { kind: 'npc', dispatch: alias, id: alias, members: [], hazards: [] }).members.push({ alias, def: section });
        } else if (aliasHeaders.length > 0) {
            looseNpc(alias, 'handler-outside-skill-scripts', section.span, ...aliasHeaders.map((header) => header.span));
        } else looseNpc(alias, 'no-handler', section.span);
    }
    for (const [name, headers] of ctx.idx.headers) {
        const match = /^oploc\d,(?!_)(.+)$/.exec(name);
        if (match && headers.every((header) => header.span.file.startsWith(FISHING_SCRIPTS))) join(match[1], { kind: 'loc', dispatch: match[1], id: match[1], members: [{ alias: match[1], def: ctx.idx.loc.get(match[1]) }], hazards: [] });
    }
    // Hazard spots link from fishing spot types and must carry the content's is_whirlpool flag.
    for (const section of npcs) {
        const link = section.params.get('whirlpool')?.[0];
        if (!link || ctx.idx.npc.get(link)?.params.get('is_whirlpool')?.[0] !== '^true') continue;
        hazardAliases.add(link);
        const category = section.props.get('category')?.[0];
        const subject = category ? subjects.get(`_${category}`) : undefined;
        if (subject && !subject.hazards.includes(link)) subject.hazards.push(link);
    }
    const rollGaps = { fish_roll: rollShape(ctx, 'fish_roll'), fish_roll_loc: rollShape(ctx, 'fish_roll_loc') };
    const equipmentGap = checkEquipmentShape(ctx);
    const intercept = rollGaps.fish_roll.intercepts || rollGaps.fish_roll_loc.intercepts ? interceptZone(ctx, 'frodi_intercept_fish') : null;
    const methods: MethodWire[] = [];
    const movementIds = new Set<number>();
    for (const subject of [...subjects.values()].sort((a, b) => a.kind.localeCompare(b.kind) || a.id.localeCompare(b.id))) {
        for (const slot of dispatchSlots(ctx, subject.kind, subject.dispatch)) {
            const visible = subject.members.filter((member) => {
                const label = member.def?.props.get(`op${slot}`)?.[0];
                return label !== undefined && label.toLowerCase() !== 'hidden';
            });
            if (visible.length === 0) continue;
            const id = `fishing.${subject.id}.op${slot}`;
            const header = ctx.idx.headers.get(`op${subject.kind}${slot},${subject.dispatch}`)![0];
            const start = ctx.idx.blocks(header.span.file).find((block) => block.span.first === header.span.first)!;
            const closure = fishingClosure(ctx, start, (next) => {
                const found = ctx.idx.headers.get(`op${subject.kind}${next},${subject.dispatch}`)?.[0];
                return found ? ctx.idx.blocks(found.span.file).find((block) => block.span.first === found.span.first) ?? null : null;
            });
            // A loc handler in the fishing scripts with no catch statement (the guild door) is not a gathering method.
            if (subject.kind === 'loc' && closure.rolls.length + closure.direct.length === 0) continue;
            const targets: TargetWire[] = visible.map((member) => ({ kind: subject.kind, id: ctx.entities.id(subject.kind, member.alias, id), op: slot, class: 'resource', respawn: known(null) }));
            for (const hazard of subject.hazards) targets.push({ kind: 'npc', id: ctx.entities.id('npc', hazard, id), op: slot, class: 'hazard', respawn: known(null) });
            if (subject.kind === 'npc') for (const target of targets) if (target.class === 'resource') movementIds.add(target.id);
            const requirementGaps = [...closure.requirementGaps];
            const effectGaps = [...closure.effectGaps];
            const level = Math.max(0, ...closure.levels.map((each) => each.level));
            const rollGap = closure.usesRoll ? rollGaps[closure.usesRoll].gap : null;
            if (rollGap) effectGaps.push(rollGap);
            if (closure.tools.size > 0 && equipmentGap) requirementGaps.push(equipmentGap);
            // Products: rolls plus rewarded direct adds; a product's level is its lowest reachable threshold.
            const found = new Map<string, number>();
            for (const each of [...closure.rolls.flatMap((roll) => [{ item: roll.f1, k: roll.k }, ...(roll.f2 ? [{ item: roll.f2, k: roll.k }] : [])]), ...closure.direct]) found.set(each.item, Math.min(found.get(each.item) ?? Infinity, each.k));
            const baits = new Set(closure.rolls.map((roll) => roll.bait));
            if (baits.size > 1) effectGaps.push(gap('bait-disagrees', header.span));
            const bait = baits.size === 1 ? [...baits][0] : null;
            const tools = new Set(closure.tools);
            for (const held of closure.held) if (held !== bait) tools.add(held);
            const declared = new Map<string, string | null>();
            for (const alias of closure.tools) {
                const equipment = equipmentBait(ctx, alias);
                if ('code' in equipment) requirementGaps.push(equipment);
                else {
                    declared.set(alias, equipment.bait);
                    if (equipment.bait !== bait) effectGaps.push(gap('bait-disagrees-with-equipment', equipment.span, header.span));
                }
            }
            if (bait && !closure.held.has(bait) && ![...declared.values()].includes(bait)) requirementGaps.push(gap('bait-unchecked', header.span));
            const products: Know<YieldWire[]> =
                found.size === 0
                    ? unknown('no-catch-statement', [spanRef(header.span)])
                    : partial([...found].map(([item, k]) => ({ item: ctx.entities.id('obj', item, `${id} product`), level: Math.max(level, k) })), effectGaps);
            const first = visible[0];
            const zoneMethod = (effect: ZoneEffect, zone: { zone: Zone; sources: Span[] } | Gap) => {
                if ('zone' in zone) zones.add(id, effect, zone.zone, zone.sources);
                else requirementGaps.push(zone);
            };
            if (closure.usesRoll && rollGaps[closure.usesRoll].intercepts && intercept) zoneMethod('yield-intercepted', intercept);
            for (const gate of closure.gates) zones.add(id, 'state-gated', gate.zone, gate.sources);
            const reqs: RequirementWire[] = level > 0 ? [{ id: `${id}.level`, source: spanRef(closure.levels[0].span), kind: 'skill', skill: SKILL.fishing, level }] : [];
            methods.push({
                id,
                skill: 'fishing',
                resources: found.size === 0 ? [] : [...found.keys()],
                op: { slot, label: first.def!.props.get(`op${slot}`)![0] },
                targets: known(targets.sort(byTarget)),
                products,
                tools: known(
                    [...tools].map((alias) => ({ item: ctx.entities.id('obj', alias, `${id} tool`), use_gate: null, wield_gate: null })),
                ),
                consumes: partial(bait ? [{ item: ctx.entities.id('obj', bait, `${id} bait`), count: 1 }] : [], effectGaps),
                requirements: partial(reqs, requirementGaps),
                spots: known(null),
                sources: [spanRef(header.span)],
            });
        }
    }
    const movements = [...movementIds].sort((a, b) => a - b).map((id) => ({ npc: id, region: regionOf(ctx, ctx.entities.alias('npc', id)) }));
    return { methods, loose, movements, hazardNpcs: [...hazardAliases].map((alias) => ctx.entities.id('npc', alias, 'fishing whirlpool')).sort((a, b) => a - b) };
}

// ---- placements ------------------------------------------------------------------------------------------------

/**
 * World rows of every method's resource targets, grouped by map file in filename order. LOC rows come from the
 * `LOC` section and NPC spawns from the `NPC` section; a malformed row fails the whole extraction.
 */
function scanPlacements(ctx: Ctx, methods: MethodWire[]): PlacementFile[] {
    const locIds = new Set<number>();
    const npcIds = new Set<number>();
    for (const method of methods) {
        if (method.spots.state !== 'known' || method.targets.state === 'unknown') continue;
        for (const target of method.targets.value) if (target.class === 'resource') (target.kind === 'loc' ? locIds : npcIds).add(target.id);
    }
    const files: PlacementFile[] = [];
    for (const map of ctx.idx.maps()) {
        const text = fs.readFileSync(path.join(ctx.idx.root, map.path), 'utf8');
        const rows = [
            ...scanMapSection(map.path, text, 'LOC', locIds).map((row) => ({ row, tag: 'l' })),
            ...scanMapSection(map.path, text, 'NPC', npcIds).map((row) => ({ row, tag: 'n' })),
        ].sort((a, b) => a.row.line - b.row.line);
        if (rows.length > 0) files.push({ file: map.path, rows: rows.map(({ row, tag }) => `${row.line} ${row.plane} ${row.lx} ${row.lz} ${tag}${row.id} ${row.angle}`) });
    }
    return files;
}

// ---- family ----------------------------------------------------------------------------------------------------

const KNOWLEDGE_CELLS = ['targets', 'products', 'tools', 'consumes', 'requirements', 'spots'] as const;

function summarize(methods: MethodWire[], mining: MiningModel, placements: PlacementFile[], zones: ZoneWire[]): GatheringSummary {
    const count = { woodcutting: 0, mining: 0, fishing: 0 };
    let partialCells = 0;
    let unknownCells = 0;
    let targets = 0;
    for (const method of methods) {
        count[method.skill] += 1;
        for (const cell of KNOWLEDGE_CELLS) {
            const value = method[cell];
            if (value.state === 'partial') partialCells += 1;
            if (value.state === 'unknown') unknownCells += 1;
        }
        if (method.targets.state !== 'unknown') targets += method.targets.value.length;
    }
    const rocks = [...mining.rocks.values()];
    const reasons: Record<string, number> = {};
    for (const rock of rocks) if (rock.class === 'unclassified') reasons[rock.gap?.code ?? 'unknown'] = (reasons[rock.gap?.code ?? 'unknown'] ?? 0) + 1;
    const byMethod: Record<string, number> = {};
    const idToMethods = new Map<string, string[]>();
    for (const method of methods) {
        if (method.spots.state !== 'known' || method.targets.state === 'unknown') continue;
        for (const target of method.targets.value) if (target.class === 'resource') (idToMethods.get(`${target.kind}:${target.id}`) ?? idToMethods.set(`${target.kind}:${target.id}`, []).get(`${target.kind}:${target.id}`)!).push(method.id);
    }
    for (const file of placements) {
        for (const row of file.rows) {
            const entity = row.split(' ')[4];
            for (const id of idToMethods.get(`${entity[0] === 'l' ? 'loc' : 'npc'}:${entity.slice(1)}`) ?? []) byMethod[id] = (byMethod[id] ?? 0) + 1;
        }
    }
    return {
        methods: count,
        rocks: {
            population: rocks.length,
            resource: rocks.filter((rock) => rock.class === 'resource').length,
            depleted: rocks.filter((rock) => rock.class === 'depleted').length,
            hazard: rocks.filter((rock) => rock.class === 'hazard').length,
            unclassified: rocks.filter((rock) => rock.class === 'unclassified').length,
            unclassified_reasons: reasons,
        },
        targets,
        placements: { rows: placements.reduce((sum, file) => sum + file.rows.length, 0), files: placements.length, by_method: byMethod },
        zones: zones.length,
        knowledge: { partial: partialCells, unknown: unknownCells },
    };
}

/** The pipeline `FamilyDraft` (`{ schema, payload }`) plus a summary for the generator's report. */
export type GatheringFamily = { schema: number; payload: GatheringFacts; summary: GatheringSummary };

/** Extract the complete gathering family from one pinned content tree. Deterministic: no clocks, no host paths. */
export function extractGatheringFamily(content: string): GatheringFamily {
    const idx = indexContent(content);
    const ctx: Ctx = { idx, entities: new Entities(idx) };
    const scale = playerScale(ctx);
    const zones = new Zones();
    const wood = extractWoodcutting(ctx, axes(ctx), scale, zones);
    const mining = extractMining(ctx, pickaxes(ctx), scale, zones);
    const fishing = extractFishing(ctx, zones);
    const methods = [...wood.methods, ...mining.methods, ...fishing.methods];
    const ids = methods.map((method) => method.id);
    if (new Set(ids).size !== ids.length) throw new Error(`gathering: duplicate method ids ${ids.filter((id, index) => ids.indexOf(id) !== index).join(', ')}`);
    if (wood.methods.length === 0 || mining.methods.length === 0 || fishing.methods.length === 0) throw new Error('gathering: no extracted methods');
    const placements = scanPlacements(ctx, methods);
    const loose = [...wood.loose, ...mining.loose, ...fishing.loose].sort((a, b) => a.skill.localeCompare(b.skill) || a.kind.localeCompare(b.kind) || a.id - b.id);
    const incidentalGemIds = miningGemIds(ctx);
    const hazardNpcs = [...new Set([...wood.hazardNpcs, ...fishing.hazardNpcs])].sort((a, b) => a - b);
    const zoneList = zones.list();
    return {
        schema: GATHERING_SCHEMA,
        payload: { entities: ctx.entities.lines(), methods, loose, zones: zoneList, movements: fishing.movements, placements, hazard_npcs: hazardNpcs, incidental_gem_ids: incidentalGemIds },
        summary: summarize(methods, mining.model, placements, zoneList),
    };
}

/** Hazard (gas) rock locs of one mining method, with the resource keys the method mines. */
export type MiningHazardWire = { resources: string[]; locs: number[] };

/**
 * The tiny slice of the family the selected core carries so the gas-rock ids never need the family decoded: every
 * mining method with hazard loc targets, in method order. Partial targets contribute their known rows, unknown ones none.
 */
export function miningHazards(facts: GatheringFacts): MiningHazardWire[] {
    return facts.methods.flatMap((method) => {
        if (method.skill !== 'mining' || method.targets.state === 'unknown') return [];
        const locs = method.targets.value.filter((target) => target.class === 'hazard' && target.kind === 'loc').map((target) => target.id);
        return locs.length === 0 ? [] : [{ resources: method.resources, locs }];
    });
}

/**
 * The one admission rule (design-gatherer §2.2 step 4, also §3 rule 1, as
 * ruled: copper/tin default work). Targets admit per row: rows without a
 * respawn fact (tutorial-gate rocks) are excluded, and the method is admitted
 * on its Known target rows, refusing iff zero remain. `tools`, `consumes`,
 * `requirements` and `spots` must be `Known`; `products` may be `Known` or
 * `Partial` whose gap codes are all incidental gem rolls. Anything else
 * refuses the method with its gap code (e.g. `woodcutting.jungle`, karambwan,
 * memberfish on 289). There is no target-gap allowlist: excluded rows never
 * block, they simply do not admit.
 */
export const ACCEPTED_PRODUCT_GAPS: readonly string[] = ['incidental-gem-roll'];

/**
 * One pinned row of the selected core's `gather_resources` slice. Woodcutting
 * and mining publish one row per resource; fishing publishes one row per
 * `(operation, tools, bait, products)` group. `methods` is the complete set of
 * catalog methods admitted by this option. `aliases` retains pre-group
 * fishing method ids so existing settings continue to resolve.
 */
export type GatherResourceWire = {
    skill: SkillName;
    method: string;
    methods: string[];
    aliases: string[];
    key: string;
    resources: string[];
    label: string;
    level: number;
    selectable: boolean;
    gap: string | null;
};

/** A target row the slice admits on: it carries a respawn fact (`Known`, even `null` for never-depleting spots). */
export function isKnownGatherTarget(target: TargetWire): boolean {
    return target.respawn.state === 'known';
}

/** First blocking gap code of one method, in design cell order; null when admitted. */
export function gatherMethodGap(method: MethodWire): string | null {
    const knownTargets = method.targets.state === 'unknown' ? [] : method.targets.value.filter(isKnownGatherTarget);
    if (knownTargets.length === 0) {
        if (method.targets.state === 'unknown') return method.targets.gap.code;
        if (method.targets.state === 'partial' && method.targets.gaps.length > 0) return method.targets.gaps[0].code;
        const excluded = method.targets.state === 'known' ? method.targets.value.find((target) => !isKnownGatherTarget(target)) : undefined;
        if (excluded !== undefined && excluded.respawn.state === 'unknown') return excluded.respawn.gap.code;
        return 'no-known-target';
    }
    const cells: Know<unknown>[] = [method.tools, method.consumes, method.requirements, method.spots];
    for (const knowledge of cells) {
        if (knowledge.state === 'unknown') return knowledge.gap.code;
        if (knowledge.state === 'partial' && knowledge.gaps.length > 0) return knowledge.gaps[0].code;
    }
    if (method.products.state === 'unknown') return method.products.gap.code;
    if (method.products.state === 'partial') {
        const blocking = method.products.gaps.find((each) => !ACCEPTED_PRODUCT_GAPS.includes(each.code));
        if (blocking !== undefined) return blocking.code;
    }
    return null;
}

/** `raw_shrimp` → `Raw shrimp`; already-spaced keys (`rune stones`) only gain a capital. */
export function humanizeResourceKey(key: string): string {
    const spaced = key.replace(/_/g, ' ');
    return spaced.charAt(0).toUpperCase() + spaced.slice(1);
}

/**
 * Display label from the method's own product facts joined to obj display
 * names: one product name, or several joined with ` / `. Methods with no
 * product rows (refused) fall back to the humanized resource keys.
 */
export function gatherResourceLabel(method: MethodWire, itemNames: ReadonlyMap<number, string>): string {
    const products = method.products.state === 'unknown' ? [] : method.products.value;
    const names = products.map((product) => itemNames.get(product.item)).filter((name) => name !== undefined && name !== '');
    if (names.length > 0) return names.join(' / ');
    return method.resources.map(humanizeResourceKey).join(' / ');
}

function factItemIds(fact: Know<{ item: number }[]>): number[] {
    return fact.state === 'unknown' ? [] : [...new Set(fact.value.map((row) => row.item))].sort((a, b) => a - b);
}

function fishingIdentity(method: MethodWire): string {
    const tools = factItemIds(method.tools);
    const bait = factItemIds(method.consumes);
    const products = factItemIds(method.products);
    if (method.op === null || method.tools.state === 'unknown' || method.consumes.state === 'unknown' || method.products.state === 'unknown') {
        return `unknown:${method.id}`;
    }
    return JSON.stringify([method.op.label, tools, bait, products]);
}

function fishingKey(method: MethodWire): string {
    const operation = method.op?.label.toLowerCase().replace(/[^a-z0-9]+/g, '_').replace(/^_+|_+$/g, '') || 'unknown';
    const tools = factItemIds(method.tools);
    const products = factItemIds(method.products);
    return `fishing.${operation}.tool_${tools.length === 0 ? 'none' : tools.join('_')}.products_${products.length === 0 ? 'none' : products.join('_')}`;
}

function fishingLabel(method: MethodWire, itemNames: ReadonlyMap<number, string>): string {
    const products = method.products.state === 'unknown' ? [] : method.products.value;
    const productNames = products
        .map((product) => itemNames.get(product.item))
        .filter((name): name is string => name !== undefined && name !== '');
    const names = productNames.length > 0
        ? productNames
        : method.resources.map(humanizeResourceKey);
    const productLabel = names.length > 3
        ? `${names.slice(0, 3).join(' / ')} +${names.length - 3} more`
        : names.join(' / ');
    const toolNames = factItemIds(method.tools).map((id) => itemNames.get(id) ?? 'Unknown tool');
    const baitNames = factItemIds(method.consumes).map((id) => itemNames.get(id) ?? 'Unknown bait');
    const op = method.op?.label ?? 'Unknown operation';
    const equipment = toolNames.length === 0 ? 'Unknown tool' : toolNames.join(' + ');
    const bait = baitNames.length === 0 ? '' : ` + ${baitNames.join(' + ')}`;
    return `${productLabel} — ${op} (${equipment}${bait})`;
}

/** Minimum product level of one method; 0 when it names no product rows. */
export function gatherMethodLevel(method: MethodWire): number {
    const products = method.products.state === 'unknown' ? [] : method.products.value;
    if (products.length === 0) return 0;
    return products.reduce((min, product) => Math.min(min, product.level), Number.POSITIVE_INFINITY);
}

/**
 * The pinned per-skill option rows the selected core carries so the UI never
 * decodes the family: every method in content order, selectable or refused
 * with its gap code. Fishing groups are keyed from stable item ids and labels
 * must be unique (including refused rows).
 */
export function gatherResources(facts: GatheringFacts, itemNames: ReadonlyMap<number, string>): GatherResourceWire[] {
    const rows: GatherResourceWire[] = [];
    for (const method of facts.methods) {
        if (method.skill === 'fishing') continue;
        const gap = gatherMethodGap(method);
        const label = gatherResourceLabel(method, itemNames);
        const level = gatherMethodLevel(method);
        const selectable = gap === null;
        for (const resource of method.resources) {
            rows.push({
                skill: method.skill,
                method: method.id,
                methods: [method.id],
                aliases: [],
                key: resource,
                resources: [...method.resources],
                label,
                level,
                selectable,
                gap,
            });
        }
    }

    const fishingGroups = new Map<string, MethodWire[]>();
    for (const method of facts.methods) {
        if (method.skill !== 'fishing') continue;
        const identity = fishingIdentity(method);
        const group = fishingGroups.get(identity);
        if (group === undefined) fishingGroups.set(identity, [method]);
        else group.push(method);
    }
    for (const members of fishingGroups.values()) {
        const methods = [...members].sort((a, b) => a.id.localeCompare(b.id));
        const representative = methods[0];
        if (representative === undefined) continue;
        const methodIds = methods.map((method) => method.id);
        const gaps = methods.map(gatherMethodGap);
        const gap = gaps.find((reason) => reason !== null) ?? null;
        rows.push({
            skill: 'fishing',
            method: representative.id,
            methods: methodIds,
            aliases: [...methodIds],
            key: fishingKey(representative),
            resources: [...new Set(methods.flatMap((method) => method.resources))].sort(),
            label: fishingLabel(representative, itemNames),
            level: Math.min(...methods.map(gatherMethodLevel)),
            selectable: gaps.every((reason) => reason === null),
            gap,
        });
    }

    const fishingRows = rows.filter((row) => row.skill === 'fishing');
    const labels = fishingRows.map((row) => row.label);
    if (new Set(labels).size !== labels.length) {
        throw new Error(`gather_resources: fishing labels are not unique: ${labels.join(' | ')}`);
    }
    const keys = fishingRows.map((row) => row.key);
    if (new Set(keys).size !== keys.length) {
        throw new Error(`gather_resources: fishing keys are not unique: ${keys.join(', ')}`);
    }
    return rows;
}
