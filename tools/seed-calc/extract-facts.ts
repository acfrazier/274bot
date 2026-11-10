// Developer-only supplement for combat rolls absent from SelectedGameData.
// Reuse the product's source parser. Keep risk classification/max-hit data in Rust's existing loader.
import fs from 'node:fs';
import path from 'node:path';
import { parsePack, parseParamDefinitions, parseRows, requireGatherText, sourceFile, walkContentFiles } from '../game-data/extractors/common.ts';
import { parseSections, spanRef, type Section } from '../game-data/extractors/gathering-content.ts';

const [content, output] = process.argv.slice(2);
if (!content || !output) throw new Error('usage: node tools/seed-calc/extract-facts.ts CONTENT OUTPUT.json');
const selected = JSON.parse(fs.readFileSync(new URL('../../crates/api/data/game-data/289.json', import.meta.url), 'utf8'));
const defaults = parseParamDefinitions(requireGatherText(content, 'scripts/skill_combat/configs/combat.param'));
const param = (section: Section, key: string): number => {
    const values = section.params.get(key);
    // Match the product's parseNpcFiles Map.set rule: the final parameter wins.
    const raw = values?.at(-1) ?? defaults.get(key)?.default;
    if (raw === undefined || !/^-?\d+$/.test(raw)) throw new Error(`${section.name}: unknown ${key}=${raw}`);
    return Number(raw);
};
const stat = (section: Section, key: string): number => {
    // Engine NpcType.ts initializes all six combat stats to one.
    const raw = section.props.get(key)?.at(-1) ?? '1';
    if (!/^\d+$/.test(raw)) throw new Error(`${section.name}: unknown ${key}=${raw}`);
    return Number(raw);
};
const load = (ext: string) => {
    const result = new Map<string, Section>();
    for (const file of walkContentFiles(path.join(content, 'scripts'), ext)) {
        const relative = path.relative(content, file);
        for (const section of parseSections(relative, fs.readFileSync(file, 'utf8'))) {
            if (result.has(section.name)) throw new Error(`duplicate ${section.name}`);
            result.set(section.name, section);
        }
    }
    return result;
};
const npcSections = load('.npc');
const objSections = load('.obj');
const bonusNames = ['stab', 'slash', 'crush'];
const allBonusNames = [...bonusNames, 'magic', 'range'];
const source = (section: Section) => spanRef(section.span);
const npcs = selected.npc_names.rows.flatMap((row: { id: number; config: string }) => {
    const section = npcSections.get(row.config);
    if (!section) return [];
    return [{ id: row.id, config: row.config, attack: stat(section, 'attack'), defence: stat(section, 'defence'),
        attack_bonus: param(section, 'attackbonus'), defence_bonus: bonusNames.map((b) => param(section, `${b}defence`)),
        hitpoints: stat(section, 'hitpoints'), strength: stat(section, 'strength'), strength_bonus: param(section, 'strengthbonus'),
        attackrate: param(section, 'attackrate'), source: source(section) }];
});
const tables = new Map(parseRows(requireGatherText(content, 'scripts/skill_combat/configs/combat.dbrow')).map((row) => [row.name, row]));
const categoryTables = new Map<string, string>();
const formula = requireGatherText(content, 'scripts/skill_combat/scripts/combat.rs2');
for (const match of formula.matchAll(/case (weapon_\w+)\s*:\s*return\((\w+)\);/g)) categoryTables.set(match[1], match[2]);
const modeCodes = new Map([['^style_melee_accurate', 0], ['^style_melee_aggressive', 1], ['^style_melee_controlled', 2], ['^style_melee_defensive', 3]]);
const aliases = new Map([...parsePack(requireGatherText(content, 'pack/obj.pack'))].map(([alias, id]) => [id, alias]));
const requirements = new Map<string, { attack: number; strength: number; defence: number; ranged: number; magic: number; quest_gated: boolean; source: string }>();
for (const file of walkContentFiles(path.join(content, 'scripts/levelrequire/scripts'), '.rs2')) {
    if (!/^tier\d+\.rs2$/.test(path.basename(file))) continue;
    const relative = path.relative(content, file);
    const lines = fs.readFileSync(file, 'utf8').split(/\r?\n/);
    for (const [index, line] of lines.entries()) {
        const match = /^\s*\[opheld2,([^\]]+)\]\s*@([a-z0-9_]+)\(([^)]*)\)/i.exec(line);
        if (!match) continue;
        const config = match[1];
        const gate = match[2];
        const stats = gate.match(/attack|strength|defence|ranged|magic/g) ?? [];
        if (stats.length === 0) continue;
        const values = match[3].split(',').map((value) => value.trim()).filter((value) => /^\d+$/.test(value)).map(Number);
        if (gate.endsWith('defence_and_strength') && values.length === 1) {
            stats.splice(0, stats.length, 'defence', 'strength');
            values.push(values[0]);
        }
        if (stats.length !== values.length) throw new Error(`${relative}:${index + 1}: unsupported wear gate ${gate}`);
        const requirement = requirements.get(config) ?? { attack: 1, strength: 1, defence: 1, ranged: 1, magic: 1, quest_gated: false, source: '' };
        stats.forEach((name, position) => {
            requirement[name as 'attack' | 'strength' | 'defence' | 'ranged' | 'magic'] = Math.max(
                requirement[name as 'attack' | 'strength' | 'defence' | 'ranged' | 'magic'],
                values[position],
            );
        });
        requirement.quest_gated ||= gate.includes('quest');
        requirement.source = `${relative}:${index + 1}`;
        requirements.set(config, requirement);
    }
}
const items = selected.items.flatMap((item: { id: number; alias?: string; wear_position: number; wear_position_2: number; wear_position_3: number }) => {
    if (item.wear_position < 0 && item.wear_position_2 < 0 && item.wear_position_3 < 0) return [];
    const config = item.alias ?? aliases.get(item.id);
    const section = config && objSections.get(config);
    if (!config || !section) return [];
    const attack_bonus = allBonusNames.map((b) => param(section, `${b}attack`));
    const defence_bonus = allBonusNames.map((b) => param(section, `${b}defence`));
    const strength_bonus = param(section, 'strengthbonus');
    const range_bonus = param(section, 'rangebonus');
    const requirement = requirements.get(config) ?? { attack: 1, strength: 1, defence: 1, ranged: 1, magic: 1, quest_gated: false, source: '' };
    if (![...attack_bonus, ...defence_bonus, strength_bonus, range_bonus].some((value) => value !== 0) && !selected.weapon_styles.some((style: { obj_id: number }) => style.obj_id === item.id)) {
        return [];
    }
    return [{
        id: item.id, config, attack_bonus, defence_bonus, strength_bonus, range_bonus,
        wear: { attack: requirement.attack, strength: requirement.strength, defence: requirement.defence, ranged: requirement.ranged, magic: requirement.magic, quest_gated: requirement.quest_gated },
        source: source(section), wear_source: requirement.source,
    }];
});
const foodSource = (relative: string, row: string) => {
    const lines = requireGatherText(content, relative).split(/\r?\n/);
    const start = lines.findIndex((line) => line.trim() === `[${row}]`);
    if (start < 0) throw new Error(`${relative}: source row ${row} missing`);
    let end = lines.findIndex((line, index) => index > start && /^\s*\[[^\]]+\]\s*$/.test(line));
    if (end < 0) end = lines.length;
    while (end > start + 1 && !lines[end - 1].trim()) end--;
    return `${relative}:${start + 1}-${end}`;
};
const foods = selected.consumption.flatMap((fact: {
    action: string;
    item: { id: number; alias: string };
    source_file: string;
    source_row: string;
    eat_delay_arg: number | null;
    skill_delay_arg: number | null;
    message_delay: number | null;
    qualification: string;
}) => {
    if (fact.action !== 'consume' || fact.qualification !== 'fixed_hp_heal'
        || fact.eat_delay_arg === null || fact.skill_delay_arg === null || fact.message_delay !== null) return [];
    const relative = path.join('scripts/player/configs/consumption', fact.source_file);
    return [{
        id: fact.item.id,
        alias: fact.item.alias,
        source: foodSource(relative, fact.source_row),
    }];
});
const weapons = selected.weapon_styles.filter((w: { category: number }) => w.category === 0 || w.category === 1).flatMap((w: { obj_id: number; category: number; style: number; attackrate: number }) => {
    const config = aliases.get(w.obj_id);
    const section = config && objSections.get(config);
    if (!section) return [];
    const attack_bonus = allBonusNames.map((b) => param(section, `${b}attack`));
    const defence_bonus = allBonusNames.map((b) => param(section, `${b}defence`));
    const strength_bonus = param(section, 'strengthbonus');
    const range_bonus = param(section, 'rangebonus');
    const requirement = requirements.get(config) ?? { attack: 1, strength: 1, defence: 1, ranged: 1, magic: 1, quest_gated: false, source: '' };
    if (w.category !== 0) {
        const ranged = selected.ranged_weapons.find((fact: { obj_id: number }) => fact.obj_id === w.obj_id);
        if (!ranged) return [];
        return [{
            id: w.obj_id, config, kind: 'ranged', attack_bonus, defence_bonus, strength_bonus, range_bonus,
            attackrate: w.attackrate, required_attack: requirement.attack, required_ranged: ranged.levelrequire,
            damage_types: [null, null, null, null], poisoned: param(section, 'poison_severity') > 0,
            source: source(section), wear_source: requirement.source,
        }];
    }
    const tier = selected.melee_weapon_tiers.find((t: { obj_id: number }) => t.obj_id === w.obj_id);
    // Quest-gated and additional-strength-gated weapons are not generic seed tools.
    if (!tier || tier.quest_gated || tier.strength > 0) return [];
    const table = tables.get(categoryTables.get(section.props.get('category')?.[0] ?? '') ?? '');
    if (!table) return [];
    const damage_types: (number | null)[] = [null, null, null, null];
    (table.values.damagestyle ?? []).forEach((values, slot) => {
        const mode = modeCodes.get(values[0]);
        const damage = bonusNames.indexOf((table.values.damagetype?.[slot]?.[0] ?? '').replace(/^\^/, '').replace(/_style$/, ''));
        if (mode !== undefined && damage >= 0 && damage_types[mode] === null) damage_types[mode] = damage;
    });
    return [{
        id: w.obj_id, config, kind: 'melee', attack_bonus, defence_bonus, strength_bonus, range_bonus,
        attackrate: w.attackrate, required_attack: Math.max(1, tier.tier), required_ranged: 1,
        damage_types, poisoned: param(section, 'poison_severity') > 0,
        source: source(section), wear_source: requirement.source,
    }];
});
fs.writeFileSync(output, `${JSON.stringify({ revision: 289, npcs, weapons, items, foods }, null, 2)}\n`);
console.log(`Extracted ${npcs.length} NPC rolls, ${weapons.length} melee/ranged weapons, ${items.length} worn items, and ${foods.length} ordinary foods to ${output}`);
