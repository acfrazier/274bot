import fs from 'node:fs';
import path from 'node:path';

const root = path.resolve(import.meta.dirname, '../..');
for (const revision of [274, 289]) {
    const file = path.join(root, `crates/api/data/game-data/${revision}.json`);
    const payload = JSON.parse(fs.readFileSync(file, 'utf8')) as {
        schema_version: number;
        revision: number;
        provenance: { engine_commit: string; content_commit: string; inputs: { path: string; sha256: string }[] };
        items: { alias: string | null; id: number; name: string | null; cost: number; certificate_link: number; certificate_template: number }[];
    };
    if (payload.schema_version !== 1 || payload.revision !== revision) throw new Error(`${revision}: schema/revision`);
    if (!payload.provenance.engine_commit || !payload.provenance.content_commit) throw new Error(`${revision}: missing provenance`);
    if (payload.provenance.inputs.length !== 2 || payload.provenance.inputs.some((input) => !input.sha256)) throw new Error(`${revision}: missing input hash`);
    if (new Set(payload.items.map((item) => item.id)).size !== payload.items.length) throw new Error(`${revision}: duplicate ids`);
    const aliased = payload.items.filter((item) => item.alias !== null);
    if (new Set(aliased.map((item) => item.alias)).size !== aliased.length) throw new Error(`${revision}: duplicate aliases`);
    const byAlias = new Map(aliased.map((item) => [item.alias as string, item]));
    for (const [alias, expectedName] of [['rune_platebody', 'Rune platebody'], ['rune_chainbody', 'Rune chainbody']] as const) {
        const item = byAlias.get(alias);
        if (!item || item.name !== expectedName) throw new Error(`${revision}: missing ${alias}`);
    }
    const sameNameDragonhide = payload.items.filter((item) => item.name === 'Dragonhide');
    if (sameNameDragonhide.length < 2 || new Set(sameNameDragonhide.map((item) => item.id)).size !== sameNameDragonhide.length) {
        throw new Error(`${revision}: same-name dragonhide identity proof`);
    }
    const custom = payload.items.filter((item) => item.alias?.includes('dragonhide') || item.alias === 'rune_platebody');
    console.log(JSON.stringify({ revision, records: payload.items.length, aliases: aliased.length, same_name_dragonhide: sameNameDragonhide.map((item) => [item.alias, item.id]), custom_count: custom.length }));
}
