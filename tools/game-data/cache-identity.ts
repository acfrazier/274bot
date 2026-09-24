import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import { execFileSync } from 'node:child_process';

// The Rust codec is the sole definition of decoded identity. This tool never
// downloads or repacks game assets. Build `cargo build -p nav --bin cache-content-id`
// first and set GAME_DATA_IDENTITY_BIN when using a non-default target directory.
export function verifyCacheIdentity(revision: number, engine: string, expected: { cache_id: string; content_id?: string }) {
    const root = path.resolve(import.meta.dirname, '../..');
    const binary = process.env.GAME_DATA_IDENTITY_BIN ?? path.join(process.env.CARGO_TARGET_DIR ?? path.join(root, 'target'), 'debug/cache-content-id');
    const snapshots = process.env[`GAME_DATA_${revision}_SNAPSHOTS`] ?? path.join(os.homedir(), '.274bot', revision === 289 ? 'unpack-289' : 'unpack');
    if (!fs.existsSync(binary)) throw new Error(`build the offline cache-content-id binary first: ${binary}`);
    const result = JSON.parse(execFileSync(binary, [String(revision), path.join(engine, 'data/pack/client'), snapshots], { encoding: 'utf8' }));
    if (result.revision !== revision || result.format !== '274DCI01' || result.cache_id !== expected.cache_id || result.content_id !== expected.content_id) {
        throw new Error(`${revision}: actual pinned cache/snapshot identity differs from generated baseline`);
    }
    return result;
}
