import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { gatherCatalogFiles } from './extractors/gather-catalog.ts';
import { assertRs2b0tPinned, requireEnvPath } from './extractors/rs2b0t-literals.ts';

/**
 * Regenerates only the rs2b0t gather catalogs (camps, fishing methods and
 * facts, fish-camp cook plans, quest junk) in `crates/api/data/game-data/`.
 * It reads the pinned rs2b0t export and needs no engine or content checkout
 * beyond the engine's `node_modules/typescript` (the AST reader), so it runs
 * where the full `generate.ts` inputs are not available. `generate.ts` writes
 * the same bytes through the same function.
 *
 *   RS2B0T=<pinned export> GAME_DATA_289_ENGINE=<dir with node_modules/typescript> \
 *     bun tools/game-data/generate-gather-catalogs.ts [--check]
 *
 * `--check` writes nothing and fails when a committed file differs.
 */
const root = path.resolve(import.meta.dirname, '../..');
const directory = path.join(root, 'crates/api/data/game-data');

export async function main(args: string[]) {
    const check = args.includes('--check');
    if (args.some((arg) => arg !== '--check')) throw new Error('usage: generate-gather-catalogs.ts [--check]');
    const rs2b0t = requireEnvPath('RS2B0T', 'the pinned rs2b0t checkout root');
    assertRs2b0tPinned(rs2b0t);
    const files = await gatherCatalogFiles(requireEnvPath('GAME_DATA_289_ENGINE', 'a root holding node_modules/typescript'), rs2b0t);
    const stale: string[] = [];
    for (const [file, bytes] of Object.entries(files)) {
        const target = path.join(directory, file);
        if (check) {
            if (!fs.existsSync(target) || fs.readFileSync(target, 'utf8') !== bytes) stale.push(file);
        } else {
            fs.writeFileSync(target, bytes);
        }
    }
    if (stale.length) throw new Error(`stale gather catalogs: ${stale.join(', ')}`);
    console.log(JSON.stringify({ files: Object.keys(files), mode: check ? 'check' : 'write' }));
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) main(process.argv.slice(2)).catch((error) => { console.error(error); process.exitCode = 1; });
