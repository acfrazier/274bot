// M-306 / M-215 gathering family: typed resource methods, rock classes, respawn facts, tools, bait, products and
// map placements, all joined from the pinned content tree. Unknown constructs become explicit gaps, never guesses.
import fs from 'node:fs';
import path from 'node:path';
import { indexContent, parseDbRows, parseCoord, parseSections, scanMapSection, spanRef, stripComment, type ContentIndex, type DbRow, type Rs2Block, type Rs2Header, type Section, type Span } from './gathering-content.ts';
import { parseBody, walkStatements, returns, type Node, type PathCond, type Stmt } from './gathering-rs2.ts';

/** Extractor schema. Bump on any change to the wire shape below; the Rust decoder pins the same number. */
export const GATHERING_SCHEMA = 3;

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
    /** States that refuse the method at runtime (checked against worn items), not gaps: the method stays selectable. */
    forbidden_states: string[];
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
            forbidden_states: [],
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
            forbidden_states: [],
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
    forbiddenStates: string[];
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
    const closure: Closure = { levels: [], tools: new Set(), held: new Set(), rolls: [], direct: [], usesRoll: null, requirementGaps: [], forbiddenStates: [], effectGaps: [], gates: [], visited: 0 };
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
        else if (cond === '~mm_wearing_greegree = true') closure.forbiddenStates.push('monkey-form');
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
// The Hemenster competition spot uses quest-owned direct inventory grants rather than fish_roll.
const HEMENSTER_FISHING_SCRIPT = 'scripts/quests/quest_fishingcompo/scripts/hemenster_fishing.rs2';
const HEMENSTER_COMP_CONSTANTS = 'scripts/quests/quest_fishingcompo/configs/quest_fishingcompo.constant';
const HEMENSTER_NORTH_SPOT = '0_41_53_sinisterfishspot';
const HEMENSTER_CARP_METHOD = `fishing.${HEMENSTER_NORTH_SPOT}.op1`;

/**
 * Prove the one quest-owned carp case from its exact dispatch, bait choice, catch branch and completion gate.
 * The normal fishing closure intentionally does not model direct quest yields or quest-state requirements.
 */
function hemensterNorthCarp(ctx: Ctx): MethodWire | null {
    const npc = ctx.idx.npc.get(HEMENSTER_NORTH_SPOT);
    const label = npc?.props.get('op1')?.[0];
    const headers = ctx.idx.headers.get(`opnpc1,${HEMENSTER_NORTH_SPOT}`) ?? [];
    const header = headers.length === 1 ? headers[0] : undefined;
    if (!npc || npc.span.file !== FISHING_NPC || !label || label.toLowerCase() === 'hidden' || !header || header.span.file !== HEMENSTER_FISHING_SCRIPT) return null;

    const dispatch = ctx.idx.blocks(HEMENSTER_FISHING_SCRIPT).find((block) => block.kind === 'opnpc1' && block.name === HEMENSTER_NORTH_SPOT && block.span.first === header.span.first);
    const sourceBlock = (kind: string, name: string) => {
        const block = uniqueBlock(ctx, kind, name);
        return block?.span.file === HEMENSTER_FISHING_SCRIPT ? block : null;
    };
    const attempt = sourceBlock('label', 'attempt_fish_hemenster');
    const baitChoice = sourceBlock('proc', 'get_hemenster_bait');
    const catchFish = sourceBlock('label', 'hemenster_catch');
    const inCompetition = sourceBlock('proc', 'in_hemenster_comp');
    const paidFee = ctx.idx.constants.get('hemenster_comp_paidfee');
    const allFishCaught = ctx.idx.constants.get('hemenster_comp_all_fish_caught');
    if (!dispatch || codeText(dispatch) !== '@attempt_fish_hemenster;' || !attempt || !baitChoice || !catchFish || !inCompetition
        || !paidFee || !allFishCaught || paidFee.span.file !== HEMENSTER_COMP_CONSTANTS || allFishCaught.span.file !== HEMENSTER_COMP_CONSTANTS
        || Number(paidFee.value) !== 1 || Number(allFishCaught.value) !== 4) return null;

    const attemptShape = flatten(attempt);
    const attemptConditions = conditions(attemptShape.nodes);
    const levelGuards = attemptConditions.map((condition) => LEVEL_LT.exec(condition)).filter((match): match is RegExpExecArray => match !== null);
    const levelGuard = levelGuards[0];
    if (levelGuards.length !== 1 || !levelGuard || Number(levelGuard[1]) !== 10) return null;
    const levelCondition = `stat(fishing) < ${levelGuard[1]}`;
    const returnsUnder = (flat: Flat[], condition: string) => flat.some(({ stmt, path }) =>
        /^return\b/.test(stmt.text) && path.some((each) => each.polarity && each.cond === condition));
    const hasRequiredConditions = (found: string[], required: string[]) => required.every((condition) => found.includes(condition));
    if (!returnsUnder(attemptShape.flat, levelCondition)
        || !hasRequiredConditions(attemptConditions, [
            '%fishingcompo >= ^fishingcompo_won_comp',
            '~in_hemenster_comp = false',
            'inv_total(inv, fishing_rod) < 1 & inv_total(inv, $bait) < 1',
            'inv_total(inv, fishing_rod) < 1',
            'inv_total(inv, $bait) < 1',
            'inv_freespace(inv) < 1 & inv_total(inv, $bait) > 1',
        ])
        || !attemptConditions.some((condition) => condition.includes(`npc_type = ${HEMENSTER_NORTH_SPOT}`) && condition.includes('%fishingcompo'))
        || !hasStmt(attemptShape.flat, 'def_namedobj $bait = ~get_hemenster_bait')
        || !hasStmt(attemptShape.flat, 'p_delay(4)')
        || !hasStmt(attemptShape.flat, '@hemenster_catch($bait)')) return null;

    const baitShape = flatten(baitChoice);
    const baitCondition = 'inv_total(inv, red_vine_worm) > 0';
    if (!conditions(baitShape.nodes).includes(baitCondition)
        || !baitShape.flat.some(({ stmt, path }) => stmt.text === 'return (red_vine_worm)' && path.some((each) => each.polarity && each.cond === baitCondition))
        || !hasStmt(baitShape.flat, 'return (fishing_bait)')) return null;

    const catchShape = flatten(catchFish);
    const carpConditions = [`npc_type = ${HEMENSTER_NORTH_SPOT}`, '$bait = red_vine_worm'];
    const under = (flat: Flat[], text: string, required: string[]) => flat.some(({ stmt, path }) =>
        stmt.text === text && required.every((condition) => path.some((each) => each.polarity && each.cond === condition)));
    const grants = catchShape.flat.map(({ stmt }) => stmt.text).filter((text) => text.startsWith('inv_add(inv,'));
    const completionCondition = '%hemenster_comp_stage = ^hemenster_comp_all_fish_caught';
    if (!hasStmt(catchShape.flat, 'inv_del(inv, $bait, 1)')
        || !under(catchShape.flat, 'inv_add(inv, raw_giant_carp, 1)', carpConditions)
        || JSON.stringify(grants) !== JSON.stringify([
            'inv_add(inv, raw_giant_carp, 1)',
            'inv_add(inv, raw_sardine, 1)',
            'inv_add(inv, raw_sardine, 1)',
            'inv_add(inv, raw_shrimp, 1)',
        ])
        || catchShape.flat.some(({ stmt }) => stmt.text.startsWith('stat_advance(fishing,'))
        || !hasStmt(catchShape.flat, '%hemenster_comp_stage = calc(%hemenster_comp_stage + 1)')
        || !under(catchShape.flat, '~bonzo_handover_catch', [completionCondition, 'npc_find(coord, bonzo, 12, 0) = true'])) return null;

    const inCompetitionShape = flatten(inCompetition);
    const paidGate = '%hemenster_comp_stage >= ^hemenster_comp_paidfee & %fishingcompo = ^fishingcompo_started';
    const compConditions = conditions(inCompetitionShape.nodes);
    if (!hasRequiredConditions(compConditions, [
        '%hemenster_comp_stage = ^hemenster_comp_not_entered',
        paidGate,
        completionCondition,
    ]) || !returnsUnder(inCompetitionShape.flat, '%hemenster_comp_stage = ^hemenster_comp_not_entered')
        || !returnsUnder(inCompetitionShape.flat, paidGate)
        || !returnsUnder(inCompetitionShape.flat, completionCondition)
        || !hasStmt(inCompetitionShape.flat, 'return (true)')
        || !under(inCompetitionShape.flat, '~bonzo_set_places', [paidGate])
        || !under(inCompetitionShape.flat, '~bonzo_handover_catch', [completionCondition])) return null;

    const levelPath = attemptShape.flat.flatMap(({ path }) => path).find((each) => each.polarity && each.cond === levelCondition);
    if (!levelPath || !ctx.idx.obj.get('fishing_rod') || !ctx.idx.obj.get('red_vine_worm') || !ctx.idx.obj.get('raw_giant_carp')) return null;
    const level = Number(levelGuard[1]);
    const targetId = ctx.entities.id('npc', HEMENSTER_NORTH_SPOT, HEMENSTER_CARP_METHOD);
    const rod = ctx.entities.id('obj', 'fishing_rod', `${HEMENSTER_CARP_METHOD} tool`);
    const worm = ctx.entities.id('obj', 'red_vine_worm', `${HEMENSTER_CARP_METHOD} bait`);
    const carp = ctx.entities.id('obj', 'raw_giant_carp', `${HEMENSTER_CARP_METHOD} product`);
    const levelSpan = line(attempt.span.file, levelPath.line);
    const provenance = [
        header.span, npc.span, attempt.span, baitChoice.span, catchFish.span, inCompetition.span,
        levelSpan, paidFee.span, allFishCaught.span,
    ].map(spanRef);
    const questGate = gap('varp-gate', attempt.span, inCompetition.span, catchFish.span, paidFee.span, allFishCaught.span);
    return {
        id: HEMENSTER_CARP_METHOD,
        skill: 'fishing',
        resources: ['raw_giant_carp'],
        op: { slot: 1, label },
        targets: known([{ kind: 'npc', id: targetId, op: 1, class: 'resource', respawn: known(null) }]),
        products: known([{ item: carp, level }]),
        tools: known([{ item: rod, use_gate: null, wield_gate: null }]),
        consumes: known([{ item: worm, count: 1 }]),
        requirements: partial([{ id: `${HEMENSTER_CARP_METHOD}.level`, source: spanRef(levelSpan), kind: 'skill', skill: SKILL.fishing, level }], [questGate]),
        spots: known(null),
        forbidden_states: [],
        sources: provenance,
    };
}


function extractFishing(ctx: Ctx, zones: Zones): { methods: MethodWire[]; loose: LooseWire[]; movements: MovementWire[]; hazardNpcs: number[] } {
    const npcs = ctx.idx.npc.all().filter((section) => section.span.file === FISHING_NPC);
    if (npcs.length === 0) throw new Error(`${FISHING_NPC}: no fishing npc types`);
    const subjects = new Map<string, FishSubject>();
    const loose: LooseWire[] = [];
    const hazardAliases = new Set<string>();
    const contestMethod = hemensterNorthCarp(ctx);
    const looseNpc = (alias: string, code: string, ...spans: Span[]) => loose.push({ skill: 'fishing', kind: 'npc', id: ctx.entities.id('npc', alias, `loose ${alias}`), class: 'unclassified', gap: gap(code, ...spans) });
    const join = (key: string, subject: FishSubject) => subjects.get(key) ?? subjects.set(key, subject).get(key)!;
    for (const section of npcs) {
        const alias = section.name;
        if (alias === HEMENSTER_NORTH_SPOT && contestMethod) continue;
        const category = section.props.get('category')?.[0];
        const aliasHeaders = [1, 2, 3, 4, 5].flatMap((slot) => ctx.idx.headers.get(`opnpc${slot},${alias}`) ?? []);
        if (category && dispatchSlots(ctx, 'npc', `_${category}`).length > 0 && aliasHeaders.length === 0) {
            join(`_${category}`, { kind: 'npc', dispatch: `_${category}`, id: category, members: [], hazards: [] }).members.push({ alias, def: section });
        } else if (category && dispatchSlots(ctx, 'npc', `_${category}`).length > 0) {
            looseNpc(alias, 'custom-handler', section.span, ...aliasHeaders.map((header) => header.span));
        } else if (aliasHeaders.length > 0 && aliasHeaders.every((header) => header.span.file.startsWith(FISHING_SCRIPTS))) {
            join(alias, { kind: 'npc', dispatch: alias, id: alias, members: [], hazards: [] }).members.push({ alias, def: section });
        } else if (alias === HEMENSTER_NORTH_SPOT && aliasHeaders.some((header) => header.span.file === HEMENSTER_FISHING_SCRIPT)) {
            looseNpc(alias, 'quest-handler-shape', section.span, ...aliasHeaders.map((header) => header.span));
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
                forbidden_states: [...new Set(closure.forbiddenStates)].sort(),
                sources: [spanRef(header.span)],
            });
        }
    }
    if (contestMethod) {
        methods.push(contestMethod);
        const target = contestMethod.targets.state === 'known' ? contestMethod.targets.value[0] : undefined;
        if (target?.kind === 'npc') movementIds.add(target.id);
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
    forbidden_states: string[];
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

/** Locale-independent ordering for stable string IDs (the verifier uses the same comparator). */
export function compareCodepoint(a: string, b: string): number {
    let aOffset = 0;
    let bOffset = 0;
    while (aOffset < a.length && bOffset < b.length) {
        const aPoint = a.codePointAt(aOffset)!;
        const bPoint = b.codePointAt(bOffset)!;
        if (aPoint !== bPoint) return aPoint < bPoint ? -1 : 1;
        aOffset += aPoint > 0xffff ? 2 : 1;
        bOffset += bPoint > 0xffff ? 2 : 1;
    }
    return aOffset === a.length ? (bOffset === b.length ? 0 : -1) : 1;
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
                forbidden_states: [...method.forbidden_states],
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
        const methods = [...members].sort((a, b) => compareCodepoint(a.id, b.id));
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
            forbidden_states: [...new Set(methods.flatMap((method) => method.forbidden_states))].sort(),
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

// ---- gather sites (design-gatherer-settings-ux §2.3) ---------------------------------------------------------
//
// Named gathering camps, generated at build time into the selected core next
// to `gather_resources`. Surface placements cluster per skill (Chebyshev gap
// 12; sprawling woodcutting components split by nearest place and re-cluster);
// each site is named by a direct fishing movement-enum stem, else the nearest
// world-map label or bank-catalog row within 16 tiles, else that place with a
// bearing+distance suffix, capped at 64. Every check below fails closed like
// the fishing labels: a content bump that breaks a join throws, never guesses.

/** One published row of the selected core's `gather_sites` slice. Rows carry no source field. */
export type GatherSiteKeyWire = { key: string; count: number };
export type GatherSiteWire = {
    id: string;
    skill: SkillName;
    label: string;
    region: RegionWire;
    keys: GatherSiteKeyWire[];
};

/** Per-skill generator report, recorded on the manifest and pinned by verify. */
export type GatherSiteSkillReport = { sites: number; direct: number; dropped: number; outside_box: number; extra: number };
export type GatherSiteReport = Record<SkillName, GatherSiteSkillReport>;
export type GatherSitesResult = { rows: GatherSiteWire[]; report: GatherSiteReport };

/** The bank-catalog place rows a site may be named by; `CatalogBank` narrows to this. */
export type GatherSiteBank = { name: string; tile: { x: number; z: number; level: number } };

const SITE_GAP = 12;
const SITE_SPRAWL_EXTENT = 48;
const SITE_NEAR = 16;
const SITE_CAP = 64;
const SITE_SURFACE_Z = 6400;
const SITE_SPECIAL_Z = 4160;
const SITE_LABELS = 'maps/labels.txt';
const SITE_PLACEMENT_FILE = /^maps\/m(\d+)_(\d+)\.jm2$/;

type SiteTile = { x: number; z: number; level: number; keys: Set<string>; ents: Set<string> };
type SitePlace = { src: string; display: string; x: number; z: number; level: number };
type SiteBox = { minX: number; minZ: number; maxX: number; maxZ: number };
type DirectName = { name: string; stem: string; movement: string; npc: number; alias: string };

const siteNorm = (text: string) => text.toLowerCase().replace(/[^a-z0-9]/g, '');
const siteSlug = (text: string) => text.toLowerCase().replace(/[^a-z0-9]+/g, '_').replace(/^_+|_+$/g, '');

/** World-map labels plus the bank-catalog rows, with the spelling vocabulary for direct names. */
function readSitePlaces(content: string, banks: GatherSiteBank[]): { places: SitePlace[]; spelling: Map<string, string> } {
    const text = fs.readFileSync(path.join(content, SITE_LABELS), 'utf8');
    const rows: { src: string; raw: string; display: string; x: number; z: number; type: number }[] = [];
    let seen = 0;
    text.split(/\r?\n/).forEach((line, index) => {
        if (!line.startsWith('=')) return;
        seen += 1;
        const cells = line.slice(1).split(',');
        const raw = cells[0];
        const x = Number(cells[1]);
        const z = Number(cells[2]);
        const type = Number(cells[3]);
        if (raw === undefined || raw === '' || !Number.isInteger(x) || !Number.isInteger(z) || !Number.isInteger(type)) {
            throw new Error(`gather_sites: ${SITE_LABELS}:${index + 1} is not a label row`);
        }
        // The walk-map display rule: `/` becomes a space and a trailing ` (...)` clause is dropped.
        rows.push({ src: `label:${seen}`, raw, display: raw.replace(/\//g, ' ').replace(/\s*\(.*\)$/, ''), x, z, type });
    });
    const spelling = new Map<string, string>();
    for (const row of [...rows, ...banks.map((bank, index) => ({ src: `bank:${index}`, raw: bank.name, display: bank.name, x: bank.tile.x, z: bank.tile.z, type: -1 }))]) {
        const key = siteNorm(row.display);
        if (!spelling.has(key)) spelling.set(key, row.display);
    }
    const places: SitePlace[] = rows
        .filter((row) => row.type <= 1 && !/^\(/.test(row.raw))
        .map((row) => ({ src: row.src, display: row.display, x: row.x, z: row.z, level: 0 }));
    banks.forEach((bank, index) => {
        if (!Number.isInteger(bank.tile.x) || !Number.isInteger(bank.tile.z) || !Number.isInteger(bank.tile.level)) {
            throw new Error(`gather_sites: bank ${bank.name} has no integer tile`);
        }
        places.push({ src: `bank:${index}`, display: bank.name, x: bank.tile.x, z: bank.tile.z, level: bank.tile.level });
    });
    return { places, spelling };
}

/** `alias -> fishing_movement_enum` plus every fishing NPC alias suffix, read with the shared section parser. */
function readFishingEnums(content: string): { enumOf: Map<string, string>; suffixes: Set<string> } {
    const sections = parseSections(FISHING_NPC, fs.readFileSync(path.join(content, FISHING_NPC), 'utf8'));
    if (sections.length === 0) throw new Error(`gather_sites: ${FISHING_NPC} has no fishing npc types`);
    const enumOf = new Map<string, string>();
    const suffixes = new Set<string>();
    for (const section of sections) {
        suffixes.add(section.name.replace(/^\d+_\d+_\d+_/, ''));
        const movement = section.params.get('fishing_movement_enum')?.[0];
        if (movement === undefined) continue;
        const prev = enumOf.get(section.name);
        if (prev !== undefined && prev !== movement) throw new Error(`gather_sites: ${FISHING_NPC} names ${section.name} twice with different movement enums`);
        enumOf.set(section.name, movement);
    }
    return { enumOf, suffixes };
}

const movementStem = (movement: string): string | null => /^fishing_movement_(.+)_enum$/.exec(movement)?.[1] ?? null;

/**
 * The geographic-stem rule (never a table): the identifier without its fixed
 * wrapper and one trailing digit must be letters and underscores only and
 * must contain no fishing NPC category suffix, which refuses coordinate-only
 * (`0_34_50`, `0_41_57_saltfish`) and resource/direction (`slimey_eel_east`)
 * identifiers without listing either.
 */
function geographicStem(stem: string, suffixes: ReadonlySet<string>): string | null {
    const bare = stem.replace(/\d$/, '');
    if (!/^[a-z]+(_[a-z]+)*$/.test(bare)) return null;
    for (const suffix of suffixes) if (bare.includes(suffix)) return null;
    return bare;
}

/** Capitalised words, taking the world-map or bank spelling when the letters match. */
function humanizeStem(stem: string, spelling: ReadonlyMap<string, string>): string {
    const words = stem.split('_').map((word) => word.charAt(0).toUpperCase() + word.slice(1)).join(' ');
    return spelling.get(siteNorm(words)) ?? words;
}

const siteExtent = (box: SiteBox) => Math.max(box.maxX - box.minX, box.maxZ - box.minZ);
const siteEdgeDist = (place: SitePlace, box: SiteBox) => Math.max(0, box.minX - place.x, place.x - box.maxX, box.minZ - place.z, place.z - box.maxZ);
const siteInBox = (tile: SiteTile, box: RegionWire) => tile.level === box.level && tile.x >= box.min_x && tile.x <= box.max_x && tile.z >= box.min_z && tile.z <= box.max_z;
/** The box a site is named by: an underground box projected onto the surface map (`z - SITE_SURFACE_Z`). */
const siteNamingBox = (box: SiteBox): SiteBox => (box.minZ >= SITE_SURFACE_Z ? { minX: box.minX, minZ: box.minZ - SITE_SURFACE_Z, maxX: box.maxX, maxZ: box.maxZ - SITE_SURFACE_Z } : box);

/** 8-way bearing from a place point to a box centre. */
function siteBearing(place: SitePlace, centre: { x: number; z: number }): string {
    const dx = centre.x - place.x;
    const dz = centre.z - place.z;
    if (dx === 0 && dz === 0) return '';
    const degrees = (Math.atan2(dz, dx) * 180) / Math.PI;
    return ['E', 'NE', 'N', 'NW', 'W', 'SW', 'S', 'SE'][Math.round(((degrees + 360) % 360) / 45) % 8]!;
}

/** Union-find at a Chebyshev gap, joined on the same level only. */
function clusterTiles(tiles: SiteTile[], gap: number): SiteTile[][] {
    const parent = tiles.map((_, index) => index);
    const find = (index: number): number => {
        const next = parent[index];
        if (next === undefined || next === index) return index;
        const root = find(next);
        parent[index] = root;
        return root;
    };
    const cells = new Map<string, number[]>();
    tiles.forEach((tile, index) => {
        const key = `${tile.level}:${tile.x >> 5}:${tile.z >> 5}`;
        const cell = cells.get(key);
        if (cell === undefined) cells.set(key, [index]);
        else cell.push(index);
    });
    tiles.forEach((tile, index) => {
        for (let dx = -1; dx <= 1; dx += 1) {
            for (let dz = -1; dz <= 1; dz += 1) {
                const cell = cells.get(`${tile.level}:${(tile.x >> 5) + dx}:${(tile.z >> 5) + dz}`) ?? [];
                for (const other of cell) {
                    const peer = tiles[other];
                    if (peer !== undefined && other > index && peer.level === tile.level && Math.max(Math.abs(tile.x - peer.x), Math.abs(tile.z - peer.z)) <= gap) {
                        parent[find(index)] = find(other);
                    }
                }
            }
        }
    });
    const comps = new Map<number, SiteTile[]>();
    tiles.forEach((tile, index) => {
        const root = find(index);
        const comp = comps.get(root);
        if (comp === undefined) comps.set(root, [tile]);
        else comp.push(tile);
    });
    return [...comps.values()];
}

function siteBoxOf(tiles: SiteTile[]): SiteBox {
    let minX = Number.POSITIVE_INFINITY;
    let minZ = Number.POSITIVE_INFINITY;
    let maxX = Number.NEGATIVE_INFINITY;
    let maxZ = Number.NEGATIVE_INFINITY;
    for (const tile of tiles) {
        minX = Math.min(minX, tile.x);
        minZ = Math.min(minZ, tile.z);
        maxX = Math.max(maxX, tile.x);
        maxZ = Math.max(maxZ, tile.z);
    }
    return { minX, minZ, maxX, maxZ };
}

/**
 * The pinned per-skill named-site rows plus the per-skill report, computed
 * from the family, the `gather_resources` rows (a site's keys are exactly the
 * picker values), the content label and fishing-NPC inputs, and the bank
 * catalog. The family wire is untouched: movement boxes are reused, never
 * re-derived.
 */
export function gatherSites(facts: GatheringFacts, resources: GatherResourceWire[], content: string, banks: GatherSiteBank[]): GatherSitesResult {
    const keyMeta = new Map<string, { skill: SkillName; label: string }>();
    for (const row of resources) {
        if (keyMeta.has(row.key)) throw new Error(`gather_sites: duplicate resource key ${row.key}`);
        keyMeta.set(row.key, { skill: row.skill, label: row.label });
    }
    const methodKeys = new Map<string, string[]>();
    for (const row of resources) {
        for (const method of row.methods) {
            const keys = methodKeys.get(method);
            if (keys === undefined) methodKeys.set(method, [row.key]);
            else if (!keys.includes(row.key)) keys.push(row.key);
        }
    }
    const methodById = new Map(facts.methods.map((method) => [method.id, method]));
    const keyOps = new Map<string, string[]>();
    for (const row of resources) {
        if (row.skill !== 'fishing') continue;
        const ops: string[] = [];
        for (const id of row.methods) {
            const op = methodById.get(id)?.op?.label ?? '?';
            if (!ops.includes(op)) ops.push(op);
        }
        keyOps.set(row.key, ops.length > 0 ? ops : ['?']);
    }
    const npcAlias = new Map<number, string>();
    for (const line of facts.entities) {
        const cells = line.split(' ');
        if (cells[0] === 'npc' && cells[1] !== undefined && cells[2] !== undefined) npcAlias.set(Number(cells[1]), cells[2]);
    }
    const movementBox = new Map<number, Know<RegionWire | null>>();
    for (const movement of facts.movements) {
        if (movementBox.has(movement.npc)) throw new Error(`gather_sites: duplicate movement row for npc ${movement.npc}`);
        movementBox.set(movement.npc, movement.region);
    }
    const { places, spelling } = readSitePlaces(content, banks);
    const labelPlaces = places.filter((place) => place.src.startsWith('label:'));
    const { enumOf, suffixes } = readFishingEnums(content);
    const directCache = new Map<number, DirectName | null>();
    const directNameOf = (npc: number): DirectName | null => {
        const cached = directCache.get(npc);
        if (cached !== undefined) return cached;
        const alias = npcAlias.get(npc);
        if (alias === undefined) throw new Error(`gather_sites: npc ${npc} has no family entity alias`);
        const movement = enumOf.get(alias) ?? null;
        const stem = movement === null ? null : movementStem(movement);
        const geo = stem === null ? null : geographicStem(stem, suffixes);
        const direct = geo === null || movement === null ? null : { name: humanizeStem(geo, spelling), stem: geo, movement, npc, alias };
        directCache.set(npc, direct);
        return direct;
    };
    // Every direct name resolves: each method-target fishing NPC whose enum
    // stem is geographic has a known, non-null movement box and a non-empty
    // name, and distinct stems humanise to distinct names. This enumerates
    // method targets, so an NPC with an entirely missing movement row throws.
    const targetNpcs = new Set<number>();
    for (const method of facts.methods) {
        if (method.skill !== 'fishing' || method.targets.state === 'unknown') continue;
        for (const target of method.targets.value) if (target.kind === 'npc' && target.class === 'resource') targetNpcs.add(target.id);
    }
    const resolvedStems = new Map<string, string>();
    for (const npc of [...targetNpcs].sort((a, b) => a - b)) {
        const direct = directNameOf(npc);
        if (direct === null) continue;
        const region = movementBox.get(npc);
        if (region === undefined || region.state !== 'known' || region.value === null) {
            throw new Error(`gather_sites: direct name ${direct.movement} for npc ${npc} (${direct.alias}) has no known movement box`);
        }
        if (direct.name === '') throw new Error(`gather_sites: direct name ${direct.movement} humanizes to nothing`);
        const prev = resolvedStems.get(direct.name);
        if (prev !== undefined && prev !== direct.stem) throw new Error(`gather_sites: direct names collide: ${prev} and ${direct.stem} both humanize to ${direct.name}`);
        resolvedStems.set(direct.name, direct.stem);
    }
    // Placements per picker key over the union of that key's methods: surface
    // only, one tile row per level, remembering the entity tags on each tile.
    const entityKeys = new Map<string, { skill: SkillName; key: string }[]>();
    for (const method of facts.methods) {
        if (method.spots.state !== 'known' || method.targets.state === 'unknown') continue;
        const keys = methodKeys.get(method.id) ?? [];
        if (keys.length === 0) continue;
        for (const target of method.targets.value) {
            if (target.class !== 'resource' || !isKnownGatherTarget(target)) continue;
            const tag = `${target.kind === 'loc' ? 'l' : 'n'}${target.id}`;
            const list = entityKeys.get(tag);
            if (list === undefined) entityKeys.set(tag, keys.map((key) => ({ skill: method.skill, key })));
            else for (const key of keys) if (!list.some((entry) => entry.skill === method.skill && entry.key === key)) list.push({ skill: method.skill, key });
        }
    }
    const perKey = new Map<string, Map<string, { x: number; z: number; level: number; ents: Set<string> }>>();
    for (const file of facts.placements) {
        const fileMatch = SITE_PLACEMENT_FILE.exec(file.file);
        if (fileMatch?.[1] === undefined || fileMatch?.[2] === undefined) throw new Error(`gather_sites: unexpected placement file ${file.file}`);
        const mx = Number(fileMatch[1]);
        const mz = Number(fileMatch[2]);
        for (const row of file.rows) {
            const cells = row.split(' ');
            const ent = cells[4];
            const plane = Number(cells[1]);
            const lx = Number(cells[2]);
            const lz = Number(cells[3]);
            if (cells.length < 5 || ent === undefined || (ent[0] !== 'l' && ent[0] !== 'n') || !Number.isInteger(plane) || !Number.isInteger(lx) || !Number.isInteger(lz)) {
                throw new Error(`gather_sites: malformed placement row ${row}`);
            }
            const tile = { x: mx * 64 + lx, z: mz * 64 + lz, level: plane };
            for (const entry of entityKeys.get(ent) ?? []) {
                let tiles = perKey.get(entry.key);
                if (tiles === undefined) {
                    tiles = new Map();
                    perKey.set(entry.key, tiles);
                }
                const id = `${tile.level}:${tile.x}:${tile.z}`;
                const placed = tiles.get(id);
                if (placed === undefined) tiles.set(id, { ...tile, ents: new Set([ent]) });
                else placed.ents.add(ent);
            }
        }
    }
    const rows: GatherSiteWire[] = [];
    const report: GatherSiteReport = {
        woodcutting: { sites: 0, direct: 0, dropped: 0, outside_box: 0, extra: 0 },
        mining: { sites: 0, direct: 0, dropped: 0, outside_box: 0, extra: 0 },
        fishing: { sites: 0, direct: 0, dropped: 0, outside_box: 0, extra: 0 },
    };
    for (const skill of ['woodcutting', 'mining', 'fishing'] as const) {
        const members = new Map<string, SiteTile>();
        for (const [key, tiles] of perKey) {
            if (keyMeta.get(key)?.skill !== skill) continue;
            for (const tile of tiles.values()) {
                const id = `${tile.level}:${tile.x}:${tile.z}`;
                const member = members.get(id);
                if (member === undefined) members.set(id, { x: tile.x, z: tile.z, level: tile.level, keys: new Set([key]), ents: new Set(tile.ents) });
                else {
                    member.keys.add(key);
                    for (const ent of tile.ents) member.ents.add(ent);
                }
            }
        }
        type Draft = { d: number; extra: boolean; label: string; idbase: string; minX: number; minZ: number; maxX: number; maxZ: number; level: number; n: number; keys: GatherSiteKeyWire[] };
        const drafts: Draft[] = [];
        // Far, underground and sprawl-fragment sites need `minTiles` tiles: a fishing spot is one tile, a tree or rock fragment three.
        const minTiles = skill === 'fishing' ? 1 : 3;
        let dropped = 0;
        let direct = 0;
        let outsideBox = 0;
        const nearestBox = (box: SiteBox, level: number): { place: SitePlace; d: number } | null => {
            let best: { place: SitePlace; d: number } | null = null;
            for (const place of places) {
                if (place.level !== level) continue;
                const d = siteEdgeDist(place, box);
                if (best === null || d < best.d || (d === best.d && compareCodepoint(place.src, best.place.src) < 0)) best = { place, d };
            }
            return best;
        };
        const nearestPoint = (tile: SiteTile): { place: SitePlace; d: number } | null => {
            let best: { place: SitePlace; d: number } | null = null;
            for (const place of places) {
                if (place.level !== tile.level) continue;
                const d = Math.max(Math.abs(tile.x - place.x), Math.abs(tile.z - place.z));
                if (best === null || d < best.d || (d === best.d && compareCodepoint(place.src, best.place.src) < 0)) best = { place, d };
            }
            return best;
        };
        // The nearest labels.txt label at any distance and on any level (labels carry no level). Far clusters and underground projections take it.
        const nearestLabel = (box: SiteBox): { place: SitePlace; d: number } | null => {
            let best: { place: SitePlace; d: number } | null = null;
            for (const place of labelPlaces) {
                const d = siteEdgeDist(place, box);
                if (best === null || d < best.d || (d === best.d && compareCodepoint(place.src, best.place.src) < 0)) best = { place, d };
            }
            return best;
        };
        // The nearest bank within SITE_CAP of a far surface box, on any floor (banks carry a level; far areas ignore it, as labels do).
        const nearestBank = (box: SiteBox): { place: SitePlace; d: number } | null => {
            let best: { place: SitePlace; d: number } | null = null;
            for (const place of places) {
                if (!place.src.startsWith('bank:')) continue;
                const d = siteEdgeDist(place, box);
                if (d > SITE_CAP) continue;
                if (best === null || d < best.d || (d === best.d && compareCodepoint(place.src, best.place.src) < 0)) best = { place, d };
            }
            return best;
        };
        // The direct name of one site: the one geographic movement-enum name
        // its NPC spawns carry, each spawn inside its own box on the same
        // level. Loc placements carry no enum and never borrow a nearby NPC's
        // name; a spawn outside its own box is counted, not named; two names
        // in one site throw.
        const directOf = (comp: SiteTile[]): DirectName | null => {
            if (skill !== 'fishing') return null;
            const names = new Map<string, DirectName>();
            for (const tile of comp) {
                for (const ent of tile.ents) {
                    if (!ent.startsWith('n')) continue;
                    const named = directNameOf(Number(ent.slice(1)));
                    if (named === null) continue;
                    const box = movementBox.get(named.npc);
                    if (box?.state !== 'known' || box.value === null || !siteInBox(tile, box.value)) {
                        outsideBox += 1;
                        continue;
                    }
                    names.set(named.name, named);
                }
            }
            if (names.size > 1) throw new Error(`gather_sites: ambiguous direct names in one site: ${[...names.keys()].join(' / ')}`);
            return names.size === 1 ? [...names.values()][0]! : null;
        };
        // A site's label and id base come from its place, or from its direct name. An underground site (box at z >= SITE_SURFACE_Z) is named from its surface projection: `<place> (underground)`, id base `<skill>.<place slug>.underground`. A far surface area (`area`) is named `<place> area` with no bearing.
        const draftOf = (comp: SiteTile[], place: SitePlace, named: DirectName | null, extra: boolean, area = false): Draft => {
            const box = siteBoxOf(comp);
            const naming = siteNamingBox(box);
            const underground = box.minZ >= SITE_SURFACE_Z;
            const d = siteEdgeDist(place, naming);
            const centre = { x: Math.round((naming.minX + naming.maxX) / 2), z: Math.round((naming.minZ + naming.maxZ) / 2) };
            const counts = new Map<string, number>();
            for (const tile of comp) for (const key of tile.keys) counts.set(key, (counts.get(key) ?? 0) + 1);
            const ordered = [...counts.entries()].sort((a, b) => b[1] - a[1] || compareCodepoint(a[0], b[0]));
            const name = named !== null ? named.name : underground ? `${place.display} (underground)` : area ? `${place.display} area` : place.display;
            const bearing = named === null && !area && d > SITE_NEAR ? siteBearing(place, centre) : '';
            const br = bearing === '' ? '' : `${bearing}${d}`;
            let contents: string;
            if (skill === 'fishing') {
                const ops: string[] = [];
                for (const [key] of ordered) for (const op of keyOps.get(key) ?? ['?']) if (!ops.includes(op)) ops.push(op);
                contents = ops.join(', ');
            } else {
                const parts = ordered.map(([key, count]) => `${keyMeta.get(key)?.label ?? key} ${count}`);
                contents = parts.slice(0, 3).join(', ') + (parts.length > 3 ? ` +${parts.length - 3}` : '');
            }
            const stem = underground ? `${siteSlug(place.display)}.underground` : siteSlug(name);
            return {
                d: named === null ? d : 0, extra, label: `${name}${br === '' ? '' : ` ${br}`} · ${contents}`,
                idbase: `${skill}.${stem}${bearing === '' ? '' : `.${bearing.toLowerCase()}`}`,
                minX: box.minX, minZ: box.minZ, maxX: box.maxX, maxZ: box.maxZ, level: comp[0]!.level, n: comp.length,
                keys: ordered.map(([key, count]) => ({ key, count })),
            };
        };
        // Far surface clusters take the nearest bank within SITE_CAP (named `<bank> area`), else the nearest label at any distance. Clusters reaching the 4160-6400 special-area band are counted, not offered.
        const farSite = (comp: SiteTile[]) => {
            const box = siteBoxOf(comp);
            if (box.maxZ >= SITE_SPECIAL_Z || comp.length < minTiles) {
                dropped += comp.length;
                return;
            }
            const bank = nearestBank(box);
            if (bank !== null) {
                drafts.push(draftOf(comp, bank.place, null, true, true));
                return;
            }
            const near = nearestLabel(box);
            if (near === null) dropped += comp.length;
            else drafts.push(draftOf(comp, near.place, null, true));
        };
        // Underground clusters take the nearest label to their surface projection, at any distance.
        const undergroundSite = (comp: SiteTile[]) => {
            const near = nearestLabel(siteNamingBox(siteBoxOf(comp)));
            if (comp.length < minTiles || near === null) dropped += comp.length;
            else drafts.push(draftOf(comp, near.place, null, true));
        };
        const surface: SiteTile[] = [];
        const underground: SiteTile[] = [];
        for (const tile of members.values()) {
            if (tile.z < SITE_SURFACE_Z) surface.push(tile);
            else underground.push(tile);
        }
        for (const comp of clusterTiles(surface, SITE_GAP)) {
            const box = siteBoxOf(comp);
            if (siteExtent(box) <= SITE_SPRAWL_EXTENT) {
                const near = nearestBox(box, comp[0]!.level);
                if (near === null || near.d > SITE_CAP) {
                    farSite(comp);
                    continue;
                }
                const named = directOf(comp);
                if (named !== null) direct += 1;
                drafts.push(draftOf(comp, near.place, named, false));
                continue;
            }
            const cells = new Map<string, { place: SitePlace; pts: SiteTile[] }>();
            const far: SiteTile[] = [];
            for (const tile of comp) {
                const near = nearestPoint(tile);
                if (near === null || near.d > SITE_CAP) {
                    far.push(tile);
                    continue;
                }
                const cell = cells.get(near.place.src);
                if (cell === undefined) cells.set(near.place.src, { place: near.place, pts: [tile] });
                else cell.pts.push(tile);
            }
            for (const { place, pts } of cells.values()) {
                for (const part of clusterTiles(pts, SITE_GAP)) {
                    if (part.length < minTiles) {
                        dropped += part.length;
                        continue;
                    }
                    // Only fishing admits a fragment under 3 tiles (rule 3): it is new, so it sorts after every 0.2.0 id.
                    drafts.push(draftOf(part, place, null, part.length < 3));
                }
            }
            const reachable = far.filter((tile) => tile.z < SITE_SPECIAL_Z);
            dropped += far.length - reachable.length;
            for (const part of clusterTiles(reachable, SITE_GAP)) farSite(part);
        }
        for (const comp of clusterTiles(underground, SITE_GAP)) undergroundSite(comp);
        const byBase = new Map<string, Draft[]>();
        for (const draft of drafts) {
            const group = byBase.get(draft.idbase);
            if (group === undefined) byBase.set(draft.idbase, [draft]);
            else group.push(draft);
        }
        for (const group of byBase.values()) {
            group.sort((a, b) => Number(a.extra) - Number(b.extra) || a.d - b.d || b.n - a.n || a.minX - b.minX || a.minZ - b.minZ);
            group.forEach((draft, index) => {
                const id = index === 0 ? draft.idbase : `${draft.idbase}.${index + 1}`;
                const label = index === 0 ? draft.label : draft.label.replace(' · ', ` (${index + 1}) · `);
                rows.push({
                    id, skill, label,
                    region: { min_x: draft.minX, min_z: draft.minZ, max_x: draft.maxX, max_z: draft.maxZ, level: draft.level },
                    keys: draft.keys,
                });
            });
        }
        report[skill] = { sites: drafts.length, direct, dropped, outside_box: outsideBox, extra: drafts.filter((draft) => draft.extra).length };
    }
    // Sort by place names so plain names precede their bearing siblings.
    rows.sort((a, b) => {
        const aSeparator = a.label.indexOf(' · ');
        const bSeparator = b.label.indexOf(' · ');
        return compareCodepoint(a.label.slice(0, aSeparator), b.label.slice(0, bSeparator))
            || compareCodepoint(a.label, b.label);
    });
    const ids = rows.map((row) => row.id);
    if (new Set(ids).size !== ids.length) throw new Error(`gather_sites: duplicate ids ${ids.filter((id, index) => ids.indexOf(id) !== index).join(', ')}`);
    for (const skill of ['woodcutting', 'mining', 'fishing'] as const) {
        const labels = rows.filter((row) => row.skill === skill).map((row) => row.label);
        if (new Set(labels).size !== labels.length) {
            throw new Error(`gather_sites: duplicate ${skill} labels ${labels.filter((label, index) => labels.indexOf(label) !== index).join(' | ')}`);
        }
    }
    for (const row of rows) {
        // Far sites take a bearing and distance after the name (`Far Place NE3110`), and counts follow ` · `. Only the name must be free of 4-digit runs (raw coordinates).
        const head = row.label.split(' · ')[0]!.replace(/ (?:NE|NW|SE|SW|N|E|S|W)\d+$/, '');
        if (/\d{4}/.test(head)) throw new Error(`gather_sites: ${row.id} label leaks a number: ${row.label}`);
        if (row.region.min_z < SITE_SURFACE_Z && row.region.max_z >= SITE_SURFACE_Z) throw new Error(`gather_sites: ${row.id} spans the surface and underground`);
        if (row.region.min_x > row.region.max_x || row.region.min_z > row.region.max_z) throw new Error(`gather_sites: ${row.id} has an inverted region`);
        if (row.keys.length === 0) throw new Error(`gather_sites: ${row.id} offers no keys`);
        for (const entry of row.keys) {
            if (entry.count <= 0) throw new Error(`gather_sites: ${row.id} has an empty key ${entry.key}`);
            if (keyMeta.get(entry.key)?.skill !== row.skill) throw new Error(`gather_sites: ${row.id} offers ${entry.key}, not a ${row.skill} key`);
        }
    }
    return { rows, report };
}
