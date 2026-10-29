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
const source = (section: Section) => `${spanRef(section.span)} sha256=${sourceFile(content, section.span.file).sha256}`;
const npcs = selected.npc_names.rows.flatMap((row: { id: number; config: string }) => {
    const section = npcSections.get(row.config);
    if (!section) return [];
    return [{ id: row.id, config: row.config, attack: stat(section, 'attack'), defence: stat(section, 'defence'),
        attack_bonus: param(section, 'attackbonus'), defence_bonus: bonusNames.map((b) => param(section, `${b}defence`)),
        hitpoints: stat(section, 'hitpoints'), strength: stat(section, 'strength'), strength_bonus: param(section, 'strengthbonus'),
        attackrate: param(section, 'attackrate'), max_dealt: param(section, 'max_dealt'), source: source(section) }];
});
const tables = new Map(parseRows(requireGatherText(content, 'scripts/skill_combat/configs/combat.dbrow')).map((row) => [row.name, row]));
const categoryTables = new Map<string, string>();
const formula = requireGatherText(content, 'scripts/skill_combat/scripts/combat.rs2');
for (const match of formula.matchAll(/case (weapon_\w+)\s*:\s*return\((\w+)\);/g)) categoryTables.set(match[1], match[2]);
const modeCodes = new Map([['^style_melee_accurate', 0], ['^style_melee_aggressive', 1], ['^style_melee_controlled', 2], ['^style_melee_defensive', 3]]);
const aliases = new Map([...parsePack(requireGatherText(content, 'pack/obj.pack'))].map(([alias, id]) => [id, alias]));
const weapons = selected.weapon_styles.filter((w: { category: number }) => w.category === 0).flatMap((w: { obj_id: number }) => {
    const config = aliases.get(w.obj_id);
    const section = config && objSections.get(config);
    const tier = selected.melee_weapon_tiers.find((t: { obj_id: number }) => t.obj_id === w.obj_id);
    // Quest-gated and additional-strength-gated weapons are not generic seed tools.
    if (!section || !tier || tier.quest_gated || tier.strength > 0) return [];
    const table = tables.get(categoryTables.get(section.props.get('category')?.[0] ?? '') ?? '');
    if (!table) return [];
    const damage_types: (number | null)[] = [null, null, null, null];
    (table.values.damagestyle ?? []).forEach((values, slot) => {
        const mode = modeCodes.get(values[0]);
        const damage = bonusNames.indexOf((table.values.damagetype?.[slot]?.[0] ?? '').replace(/^\^/, '').replace(/_style$/, ''));
        if (mode !== undefined && damage >= 0 && damage_types[mode] === null) damage_types[mode] = damage;
    });
    return [{ id: w.obj_id, config, attack_bonus: bonusNames.map((b) => param(section, `${b}attack`)),
        defence_bonus: bonusNames.map((b) => param(section, `${b}defence`)), strength_bonus: param(section, 'strengthbonus'),
        attackrate: param(section, 'attackrate'), required_attack: Math.max(1, tier.tier), damage_types,
        poisoned: param(section, 'poison_severity') > 0, source: source(section) }];
});
fs.writeFileSync(output, `${JSON.stringify({ revision: 289, npcs, weapons }, null, 2)}\n`);
console.log(`Extracted ${npcs.length} NPC roll records and ${weapons.length} generic melee weapons to ${output}`);
