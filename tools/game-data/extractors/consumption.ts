import { parseRows } from './common.ts';
import { stripComment } from './gathering-content.ts';

type DelayRule = {
    base: number | null;
    when_next_stage?: { stage: string | null; value: number | null };
};
export type ConsumptionRule = {
    effect: string;
    eat_delay_arg: DelayRule;
    skill_delay_arg: DelayRule;
};
export type ConsumptionEffects = {
    aliases: ReadonlyMap<string, ConsumptionRule>;
    categories: ReadonlyMap<string, ConsumptionRule>;
};

function sameDelay(left: DelayRule, right: DelayRule) {
    return left.base === right.base
        && left.when_next_stage?.stage === right.when_next_stage?.stage
        && left.when_next_stage?.value === right.when_next_stage?.value;
}

function addRule(map: Map<string, ConsumptionRule>, key: string, rule: ConsumptionRule, label: string) {
    const current = map.get(key);
    if (current !== undefined && (current.effect !== rule.effect
        || !sameDelay(current.eat_delay_arg, rule.eat_delay_arg)
        || !sameDelay(current.skill_delay_arg, rule.skill_delay_arg))) {
        throw new Error(`${label}: conflicting consumption rules`);
    }
    map.set(key, rule);
}

function literalDelay(raw: string, label: string): number | null {
    const value = raw.trim().replace(/^\^/, '');
    if (value === 'null') return null;
    if (!/^-?\d+$/.test(value)) throw new Error(`${label}: unsupported delay argument ${raw}`);
    const parsed = Number(value);
    if (!Number.isSafeInteger(parsed) || parsed < 0) throw new Error(`${label}: invalid delay argument ${raw}`);
    return parsed;
}

function delayRule(raw: string, body: string, label: string): DelayRule {
    const value = raw.trim();
    if (!value.startsWith('$')) return { base: literalDelay(value, label) };
    const variable = value.slice(1);
    if (!/^[A-Za-z][A-Za-z0-9_]*$/.test(variable)) throw new Error(`${label}: invalid delay variable ${raw}`);
    const normalized = body.replace(/\s+/g, ' ');
    const definition = new RegExp(`\\bdef_int\\s+\\$${variable}\\s*=\\s*(-?\\d+)\\b`).exec(normalized);
    if (!definition) throw new Error(`${label}: missing integer definition for ${raw}`);
    const base = literalDelay(definition[1]!, label);
    const condition = new RegExp(`\\bif\\s*\\(\\s*oc_param\\(\\s*last_item\\s*,\\s*next_obj_stage\\s*\\)\\s*=\\s*(\\^?[A-Za-z][A-Za-z0-9_]*|null)\\s*\\)\\s*\\{\\s*\\$${variable}\\s*=\\s*(-?\\d+)\\s*;?\\s*\\}`).exec(normalized);
    if (!condition) return { base };
    return {
        base,
        when_next_stage: {
            stage: condition[1] === 'null' ? null : condition[1]!.replace(/^\^/, ''),
            value: literalDelay(condition[2]!, label),
        },
    };
}

function ruleFromBranch(body: string, label: string): ConsumptionRule | null {
    const calls = [...body.matchAll(/@player_consume_item\s*\(([^()]*)\)/g)];
    if (calls.length > 1) throw new Error(`${label}: multiple player_consume_item effects in one branch`);
    if (calls.length === 0) return null;
    const args = calls[0]![1]!.split(',').map((value) => value.trim());
    if (args.length !== 3 || !/^\^?[A-Za-z][A-Za-z0-9_]*$/.test(args[0]!)) {
        throw new Error(`${label}: malformed player_consume_item call`);
    }
    return {
        effect: args[0]!.replace(/^\^/, ''),
        eat_delay_arg: delayRule(args[1]!, body, label),
        skill_delay_arg: delayRule(args[2]!, body, label),
    };
}

function resolveDelay(rule: DelayRule, nextStage: string | null) {
    return rule.when_next_stage && rule.when_next_stage.stage === nextStage
        ? rule.when_next_stage.value
        : rule.base;
}

function addMessageDelay(map: Map<string, number>, alias: string, delay: number, label: string) {
    const current = map.get(alias);
    if (current !== undefined && current !== delay) throw new Error(`${label}: conflicting message delays ${current} and ${delay}`);
    map.set(alias, delay);
}

type Trigger = { target: string; lines: string[] };
type Case = { labels: string[]; lines: string[] };


function sectionCases(body: string) {
    const cases: Case[] = [];
    let current: Case | null = null;
    const flush = () => {
        if (current) cases.push(current);
        current = null;
    };
    for (const line of body.split(/\r?\n/)) {
        const match = /^\s*case\s+(.+?)\s*:\s*(.*)$/.exec(line);
        if (match) {
            flush();
            current = { labels: match[1]!.split(',').map((value) => value.trim()).filter(Boolean), lines: [match[2]!] };
        } else if (current) {
            current.lines.push(line);
        }
    }
    flush();
    return cases;
}

/** Parse item/category handlers from consume.rs2, including literal delay arguments. */
export function parseConsumptionEffects(text: string): ConsumptionEffects {
    const aliases = new Map<string, ConsumptionRule>();
    const categories = new Map<string, ConsumptionRule>();
    let current: Trigger | null = null;
    const flush = () => {
        if (!current) return;
        const body = current.lines.join('\n');
        const label = `consume.rs2 [opheld1,${current.target}]`;
        const destination = current.target.startsWith('_') ? categories : aliases;
        const key = current.target.startsWith('_') ? current.target.slice(1) : current.target;
        if (/\bswitch_obj\s*\(/.test(body)) {
            for (const branch of sectionCases(body)) {
                const branchLabel = `${label} case ${branch.labels.join(',')}`;
                const rule = ruleFromBranch(branch.lines.join('\n'), branchLabel);
                if (!rule) continue;
                for (const name of branch.labels) {
                    if (name === 'default') addRule(categories, key, rule, `${label} default`);
                    else addRule(aliases, name, rule, branchLabel);
                }
            }
        } else {
            const rule = ruleFromBranch(body, label);
            if (rule) addRule(destination, key, rule, label);
        }
        current = null;
    };

    for (const raw of text.split(/\r?\n/)) {
        const line = stripComment(raw);
        const header = /^\s*\[\s*([A-Za-z0-9_]+)(?:\s*,\s*([^\]]+))?\]\s*(.*)$/.exec(line);
        if (header) {
            flush();
            if (header[1] === 'opheld1' && header[2]) {
                current = { target: header[2].trim(), lines: [header[3] ?? ''] };
            }
        } else if (current) {
            current.lines.push(line);
        }
    }
    flush();
    return { aliases, categories };
}

/** Resolve selected handler delays; DB-only items without a handler are unsupported. */
export function consumptionRule(
    effects: ConsumptionEffects,
    alias: string,
    category: string | undefined,
    nextStage: string | null,
) {
    const rule = effects.aliases.get(alias) ?? (category === undefined ? undefined : effects.categories.get(category));
    if (!rule) return null;
    return {
        effect: rule.effect,
        eat_delay_arg: resolveDelay(rule.eat_delay_arg, nextStage),
        skill_delay_arg: resolveDelay(rule.skill_delay_arg, nextStage),
    };
}

/** Per-item message delays passed to p_delay by consume_effects.rs2. */
export function parseConsumeMessageDelays(text: string): ReadonlyMap<string, number> {
    const delays = new Map<string, number>();
    for (const row of parseRows(text)) {
        const raw = row.values.message_delay?.[0]?.[0];
        if (raw === undefined) continue;
        const delay = literalDelay(raw, `consume_messages.dbrow/${row.name}`);
        if (delay === null) throw new Error(`consume_messages.dbrow/${row.name}: message_delay must be an integer`);
        for (const [alias] of row.values.consumable ?? []) {
            if (!alias) throw new Error(`consume_messages.dbrow/${row.name}: empty consumable alias`);
            addMessageDelay(delays, alias, delay, `consume_messages.dbrow/${row.name}/${alias}`);
        }
    }
    return delays;
}
